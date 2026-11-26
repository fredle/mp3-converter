//! Carrying tags (title, artist, album and so on) from the original to the
//! converted file, through Windows' own property handlers: the same ones
//! Explorer uses for its Details pane. Best effort: a format or Windows
//! edition without a handler simply gets no tags. Album art, which the
//! handlers can show but not write, is carried by `art`.

pub mod art;

use std::path::Path;

use windows::Win32::Foundation::PROPERTYKEY;
use windows::Win32::Storage::EnhancedStorage::{
    PKEY_Media_Year, PKEY_Music_AlbumArtist, PKEY_Music_AlbumTitle, PKEY_Music_Artist,
    PKEY_Music_Genre, PKEY_Music_TrackNumber, PKEY_Title,
};
use windows::Win32::System::Com::StructuredStorage::PropVariantClear;
use windows::Win32::System::Com::{COINIT_MULTITHREADED, CoInitializeEx, IBindCtx};
use windows::Win32::System::Variant::VT_EMPTY;
use windows::Win32::UI::Shell::PropertiesSystem::{
    GETPROPERTYSTOREFLAGS, GPS_DEFAULT, GPS_READWRITE, IPropertyStore,
    SHGetPropertyStoreFromParsingName,
};
use windows::core::HSTRING;

/// The tags copied, in the order Explorer lists them.
const KEYS: [PROPERTYKEY; 7] = [
    PKEY_Title,
    PKEY_Music_Artist,
    PKEY_Music_AlbumArtist,
    PKEY_Music_AlbumTitle,
    PKEY_Media_Year,
    PKEY_Music_TrackNumber,
    PKEY_Music_Genre,
];

fn store(path: &Path, flags: GETPROPERTYSTOREFLAGS) -> Option<IPropertyStore> {
    unsafe {
        // Reference counted per thread and safe to repeat.
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
        SHGetPropertyStoreFromParsingName(&HSTRING::from(path.as_os_str()), None::<&IBindCtx>, flags)
            .ok()
    }
}

/// Copies whichever of the tags `source` has onto `dest` and returns how
/// many were written. Any failure just means fewer tags; it never touches
/// the audio.
pub fn copy(source: &Path, dest: &Path) -> usize {
    let Some(from) = store(source, GPS_DEFAULT) else {
        return 0;
    };
    let Some(to) = store(dest, GPS_READWRITE) else {
        return 0;
    };
    let mut copied = 0;
    for key in &KEYS {
        unsafe {
            let Ok(mut value) = from.GetValue(key) else {
                continue;
            };
            // Same key on both sides, so the value's type fits as it is.
            if value.Anonymous.Anonymous.vt != VT_EMPTY && to.SetValue(key, &value).is_ok() {
                copied += 1;
            }
            let _ = PropVariantClear(&mut value);
        }
    }
    if copied == 0 {
        return 0;
    }
    match unsafe { to.Commit() } {
        Ok(()) => copied,
        Err(_) => 0,
    }
}

/// Writes one text tag, as a list for multi-value tags such as the artist.
/// For tests: the app itself only copies.
#[cfg(test)]
pub fn set_text(path: &Path, key: &PROPERTYKEY, text: &str, list: bool) -> bool {
    use windows::Win32::System::Com::CoTaskMemAlloc;
    use windows::Win32::System::Com::StructuredStorage::{
        InitPropVariantFromStringVector, PROPVARIANT,
    };
    use windows::Win32::System::Variant::VT_LPWSTR;
    use windows::core::{PCWSTR, PWSTR};

    let Some(store) = store(path, GPS_READWRITE) else {
        return false;
    };
    let wide = HSTRING::from(text);
    unsafe {
        let mut value = if list {
            match InitPropVariantFromStringVector(Some(&[PCWSTR(wide.as_ptr())])) {
                Ok(value) => value,
                Err(_) => return false,
            }
        } else {
            let units: &[u16] = &wide;
            let buffer = CoTaskMemAlloc((units.len() + 1) * 2) as *mut u16;
            if buffer.is_null() {
                return false;
            }
            std::ptr::copy_nonoverlapping(units.as_ptr(), buffer, units.len());
            *buffer.add(units.len()) = 0;
            let mut value = PROPVARIANT::default();
            let inner = &mut *value.Anonymous.Anonymous;
            inner.vt = VT_LPWSTR;
            inner.Anonymous.pwszVal = PWSTR(buffer);
            value
        };
        let ok = store.SetValue(key, &value).is_ok() && store.Commit().is_ok();
        let _ = PropVariantClear(&mut value);
        ok
    }
}

/// Reads one tag as text (lists joined with "; "), for tests.
#[cfg(test)]
pub fn get_text(path: &Path, key: &PROPERTYKEY) -> Option<String> {
    use windows::Win32::System::Com::CoTaskMemFree;
    use windows::Win32::System::Com::StructuredStorage::PropVariantToStringAlloc;

    let store = store(path, GPS_DEFAULT)?;
    unsafe {
        let mut value = store.GetValue(key).ok()?;
        let text = PropVariantToStringAlloc(&value).ok();
        let _ = PropVariantClear(&mut value);
        let text = text?;
        let result = text.to_string().ok();
        CoTaskMemFree(Some(text.0 as *const _));
        result.filter(|t| !t.is_empty())
    }
}

/// Whether Windows finds an embedded picture in the file (the one Explorer
/// shows as its thumbnail), for tests.
#[cfg(test)]
pub fn windows_sees_art(path: &Path) -> bool {
    use windows::Win32::Storage::EnhancedStorage::PKEY_ThumbnailStream;
    use windows::Win32::System::Variant::VT_STREAM;
    let Some(store) = store(path, GPS_DEFAULT) else {
        return false;
    };
    unsafe {
        let Ok(mut value) = store.GetValue(&PKEY_ThumbnailStream) else {
            return false;
        };
        let found = value.Anonymous.Anonymous.vt == VT_STREAM;
        let _ = PropVariantClear(&mut value);
        found
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_files_copy_nothing_and_do_not_panic() {
        let nowhere = Path::new("C:/no/such/folder/a.mp3");
        assert_eq!(copy(nowhere, nowhere), 0);
        assert_eq!(get_text(nowhere, &PKEY_Title), None);
    }
}
