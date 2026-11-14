//! Decoding through the Windows Imaging Component, which is how HEIC/HEIF,
//! AVIF and JPEG XL are read: Windows (and its Store codec extensions) own
//! the decoders, so the app ships none of its own.

use std::path::Path;

use windows::Win32::Foundation::GENERIC_READ;
use windows::Win32::Graphics::Imaging::{
    CLSID_WICImagingFactory, GUID_WICPixelFormat32bppRGBA, IWICImagingFactory,
    WICBitmapDitherTypeNone, WICBitmapPaletteTypeCustom, WICDecodeMetadataCacheOnDemand,
};
use windows::Win32::System::Com::{
    CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED, CoCreateInstance, CoInitializeEx,
};
use windows::core::HSTRING;

/// WINCODEC_ERR_COMPONENTNOTFOUND: no installed codec recognises the file.
const COMPONENT_NOT_FOUND: u32 = 0x8898_2F50;
/// MF_E_TOPO_CODEC_NOT_FOUND: the HEIF container opened but the HEVC (or
/// AV1) video decoder it needs is not installed.
const CODEC_NOT_FOUND: u32 = 0xC00D_5212;

pub struct WicError {
    /// True when the failure means a codec extension is missing, as opposed
    /// to a damaged or unreadable file.
    pub missing_codec: bool,
    pub message: String,
}

impl From<windows::core::Error> for WicError {
    fn from(e: windows::core::Error) -> Self {
        Self {
            missing_codec: is_missing_codec(e.code().0 as u32),
            message: e.message(),
        }
    }
}

pub fn is_missing_codec(hresult: u32) -> bool {
    hresult == COMPONENT_NOT_FOUND || hresult == CODEC_NOT_FOUND
}

/// Decodes the first frame of `path` to straight-alpha RGBA.
pub fn decode(path: &Path) -> Result<image::RgbaImage, WicError> {
    unsafe {
        // Each worker thread needs COM; a repeat call on the same thread is
        // harmless, so the result is not checked.
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
        let factory: IWICImagingFactory =
            CoCreateInstance(&CLSID_WICImagingFactory, None, CLSCTX_INPROC_SERVER)?;
        let decoder = factory.CreateDecoderFromFilename(
            &HSTRING::from(path.as_os_str()),
            None,
            GENERIC_READ,
            WICDecodeMetadataCacheOnDemand,
        )?;
        let frame = decoder.GetFrame(0)?;
        let converter = factory.CreateFormatConverter()?;
        converter.Initialize(
            &frame,
            &GUID_WICPixelFormat32bppRGBA,
            WICBitmapDitherTypeNone,
            None,
            0.0,
            WICBitmapPaletteTypeCustom,
        )?;
        let (mut width, mut height) = (0u32, 0u32);
        converter.GetSize(&mut width, &mut height)?;
        let stride = width.checked_mul(4).ok_or_else(too_large)?;
        let len = (stride as usize)
            .checked_mul(height as usize)
            .ok_or_else(too_large)?;
        let mut pixels = vec![0u8; len];
        converter.CopyPixels(std::ptr::null(), stride, &mut pixels)?;
        image::RgbaImage::from_raw(width, height, pixels).ok_or_else(too_large)
    }
}

fn too_large() -> WicError {
    WicError {
        missing_codec: false,
        message: "The image is too large to convert.".into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recognises_missing_codec_results() {
        assert!(is_missing_codec(0x8898_2F50));
        assert!(is_missing_codec(0xC00D_5212));
        assert!(!is_missing_codec(0x8007_0002)); // file not found
    }
}
