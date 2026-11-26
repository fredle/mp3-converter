//! Carrying the album art (cover picture) over to the converted file.
//!
//! Windows' property handlers can show embedded art but not write it, so
//! this reads and writes the three places it lives directly: the ID3 tag
//! of an MP3 (an APIC frame), the iTunes-style `covr` item of an MP4/M4A,
//! and the PICTURE block of a FLAC file. Only JPEG and PNG pictures are
//! carried, as those are what players show. Everything is best effort:
//! anything unexpected means no art, never a failed conversion.

use std::fs::File;
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::Path;

/// An embedded picture.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Picture {
    pub mime: &'static str,
    pub data: Vec<u8>,
}

/// The largest picture carried (a FLAC block holds at most 16 MB, and no
/// cover needs more).
const MAX_PICTURE: usize = 16 * 1024 * 1024 - 1024;
/// The most read from a file's header looking for art.
const MAX_HEADER: u64 = 64 * 1024 * 1024;

/// The picture type, from its first bytes. Only JPEG and PNG are carried.
pub fn image_mime(data: &[u8]) -> Option<&'static str> {
    if data.starts_with(&[0xFF, 0xD8, 0xFF]) {
        Some("image/jpeg")
    } else if data.starts_with(&[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A]) {
        Some("image/png")
    } else {
        None
    }
}

fn picture(data: &[u8]) -> Option<Picture> {
    if data.len() > MAX_PICTURE {
        return None;
    }
    Some(Picture {
        mime: image_mime(data)?,
        data: data.to_vec(),
    })
}

/// Copies the art of `source`, if it has any, onto `dest` (an MP3, M4A or
/// FLAC file). Returns whether art was written.
pub fn copy(source: &Path, dest: &Path) -> bool {
    match read(source) {
        Some(picture) => write(dest, &picture),
        None => false,
    }
}

/// The embedded cover of an MP3 (or anything with an ID3v2 tag), MP4
/// family (MP4, M4A, M4B, MOV, 3GP) or FLAC file.
pub fn read(path: &Path) -> Option<Picture> {
    let mut file = File::open(path).ok()?;
    let mut head = [0u8; 12];
    file.read_exact(&mut head).ok()?;
    file.seek(SeekFrom::Start(0)).ok()?;
    match Kind::of(&head)? {
        Kind::Id3 => {
            let tag = read_id3_tag(&mut file)?;
            id3_picture(&tag)
        }
        Kind::Flac => {
            let (blocks, _) = read_flac_head(&mut file)?;
            flac_picture(&blocks)
        }
        Kind::Mp4 => {
            let (moov, _) = read_moov(&mut file)?;
            mp4_picture(&moov)
        }
    }
}

/// Puts `picture` into `path` as its front cover, unless it already has
/// one. Returns whether it was written.
pub fn write(path: &Path, picture: &Picture) -> bool {
    write_inner(path, picture).unwrap_or(false)
}

fn write_inner(path: &Path, picture: &Picture) -> Option<bool> {
    let mut file = File::open(path).ok()?;
    let mut head = [0u8; 12];
    let read = file.read(&mut head).ok()?;
    file.seek(SeekFrom::Start(0)).ok()?;
    let is_mp3 = path
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("mp3"));
    let (at, old_len, new) = match Kind::of(&head[..read]) {
        Some(Kind::Id3) => {
            let tag = read_id3_tag(&mut file)?;
            (0, tag.len() as u64, id3_with_picture(Some(&tag), picture)?)
        }
        // An MP3 without a tag yet: put a new one in front.
        None if is_mp3 => (0, 0, id3_with_picture(None, picture)?),
        Some(Kind::Flac) => {
            let (blocks, len) = read_flac_head(&mut file)?;
            (4, len, flac_with_picture(&blocks, picture)?)
        }
        Some(Kind::Mp4) => {
            let (moov, at) = read_moov(&mut file)?;
            let media_after = mdat_after(&mut file, at)?;
            let new = mp4_with_picture(&moov, picture, media_after)?;
            (at, moov.len() as u64, new)
        }
        None => return None,
    };
    drop(file);
    Some(splice(path, at, old_len, &new).is_ok())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    Id3,
    Flac,
    Mp4,
}

impl Kind {
    fn of(head: &[u8]) -> Option<Kind> {
        if head.starts_with(b"ID3") {
            Some(Kind::Id3)
        } else if head.starts_with(b"fLaC") {
            Some(Kind::Flac)
        } else if head.len() >= 8
            && [&b"ftyp"[..], b"moov", b"mdat", b"wide", b"free", b"skip"].contains(&&head[4..8])
        {
            Some(Kind::Mp4)
        } else {
            None
        }
    }
}

/// Replaces `old_len` bytes at `at` with `new`: in place when the size is
/// the same or the replaced part runs to the end of the file, otherwise
/// through a copy that then takes the file's place.
fn splice(path: &Path, at: u64, old_len: u64, new: &[u8]) -> std::io::Result<()> {
    let len = std::fs::metadata(path)?.len();
    if new.len() as u64 == old_len || at + old_len == len {
        let mut file = File::options().write(true).open(path)?;
        if new.len() as u64 != old_len {
            file.set_len(at)?;
        }
        file.seek(SeekFrom::Start(at))?;
        file.write_all(new)?;
        return file.flush();
    }
    let mut name = path.as_os_str().to_owned();
    name.push(".art-tmp");
    let temp = std::path::PathBuf::from(name);
    let result = (|| {
        let mut from = File::open(path)?;
        let mut to = std::io::BufWriter::new(File::create(&temp)?);
        std::io::copy(&mut (&mut from).take(at), &mut to)?;
        to.write_all(new)?;
        from.seek(SeekFrom::Start(at + old_len))?;
        std::io::copy(&mut from, &mut to)?;
        to.flush()?;
        drop(to);
        drop(from);
        std::fs::rename(&temp, path)
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&temp);
    }
    result
}

fn be32(b: &[u8]) -> u32 {
    u32::from_be_bytes([b[0], b[1], b[2], b[3]])
}

// ---- ID3v2 (MP3) ----

fn syncsafe(b: &[u8]) -> u32 {
    (b[0] as u32 & 0x7F) << 21
        | (b[1] as u32 & 0x7F) << 14
        | (b[2] as u32 & 0x7F) << 7
        | (b[3] as u32 & 0x7F)
}

fn to_syncsafe(n: u32) -> [u8; 4] {
    [
        (n >> 21 & 0x7F) as u8,
        (n >> 14 & 0x7F) as u8,
        (n >> 7 & 0x7F) as u8,
        (n & 0x7F) as u8,
    ]
}

/// The whole ID3v2 tag at the start of the file, header included.
fn read_id3_tag(file: &mut File) -> Option<Vec<u8>> {
    let mut header = [0u8; 10];
    file.read_exact(&mut header).ok()?;
    if !header.starts_with(b"ID3") {
        return None;
    }
    let size = syncsafe(&header[6..10]) as usize;
    let footer = if header[3] == 4 && header[5] & 0x10 != 0 {
        10
    } else {
        0
    };
    let mut tag = header.to_vec();
    tag.resize(10 + size + footer, 0);
    file.read_exact(&mut tag[10..]).ok()?;
    Some(tag)
}

/// Undoes ID3 "unsynchronisation": every FF 00 was written for FF.
fn unsync(data: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(data.len());
    let mut i = 0;
    while i < data.len() {
        out.push(data[i]);
        if data[i] == 0xFF && data.get(i + 1) == Some(&0) {
            i += 1;
        }
        i += 1;
    }
    out
}

/// One frame of an ID3v2 tag: (id, flags, body).
struct Frame<'a> {
    id: &'a [u8],
    flags: [u8; 2],
    body: &'a [u8],
}

/// The frames of a tag body (after the header and any extended header),
/// stopping at the padding.
fn id3_frames(body: &[u8], major: u8) -> Vec<Frame<'_>> {
    let (id_len, header_len) = if major == 2 { (3, 6) } else { (4, 10) };
    let mut frames = Vec::new();
    let mut at = 0;
    while at + header_len <= body.len() {
        let id = &body[at..at + id_len];
        if !id
            .iter()
            .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit())
        {
            break;
        }
        let size = match major {
            2 => {
                (body[at + 3] as usize) << 16 | (body[at + 4] as usize) << 8 | body[at + 5] as usize
            }
            3 => be32(&body[at + 4..]) as usize,
            _ => syncsafe(&body[at + 4..]) as usize,
        };
        let flags = if major == 2 {
            [0, 0]
        } else {
            [body[at + 8], body[at + 9]]
        };
        let start = at + header_len;
        let Some(end) = start.checked_add(size).filter(|e| *e <= body.len()) else {
            break;
        };
        frames.push(Frame {
            id,
            flags,
            body: &body[start..end],
        });
        at = end;
    }
    frames
}

/// Where the frames start in a tag (after the header and extended
/// header), and its version.
fn id3_layout(tag: &[u8]) -> Option<(u8, usize)> {
    if tag.len() < 10 || !tag.starts_with(b"ID3") {
        return None;
    }
    let major = tag[3];
    if !(2..=4).contains(&major) {
        return None;
    }
    let mut start = 10;
    if tag[5] & 0x40 != 0 && major >= 3 {
        let ext = tag.get(10..14)?;
        start += if major == 3 {
            4 + be32(ext) as usize
        } else {
            syncsafe(ext) as usize
        };
    }
    (start <= tag.len()).then_some((major, start))
}

/// The front cover of an ID3v2 tag (or else its first picture).
pub fn id3_picture(tag: &[u8]) -> Option<Picture> {
    let (major, start) = id3_layout(tag)?;
    let size = syncsafe(&tag[6..10]) as usize;
    let end = (10 + size).min(tag.len());
    let raw = tag.get(start..end)?;
    let whole_unsync = tag[5] & 0x80 != 0 && major < 4;
    let body = if whole_unsync {
        unsync(raw)
    } else {
        raw.to_vec()
    };
    let mut first = None;
    for frame in id3_frames(&body, major) {
        if frame.id != b"APIC" && frame.id != b"PIC" {
            continue;
        }
        let mut data = frame.body;
        let unsynced;
        match major {
            3 if frame.flags[1] & 0xC0 != 0 => continue,
            3 if frame.flags[1] & 0x20 != 0 => data = data.get(1..)?,
            4 => {
                if frame.flags[1] & 0x0C != 0 {
                    continue;
                }
                if frame.flags[1] & 0x01 != 0 {
                    data = data.get(4..)?;
                }
                if frame.flags[1] & 0x02 != 0 {
                    unsynced = unsync(data);
                    data = &unsynced;
                }
            }
            _ => {}
        }
        let Some((kind, image)) = apic_image(data, major == 2) else {
            continue;
        };
        let Some(picture) = picture(image) else {
            continue;
        };
        if kind == 3 {
            return Some(picture);
        }
        first.get_or_insert(picture);
    }
    first
}

/// (picture type, image bytes) of an APIC (or v2.2 PIC) frame body.
fn apic_image(body: &[u8], v22: bool) -> Option<(u8, &[u8])> {
    let encoding = *body.first()?;
    let mut at = 1;
    if v22 {
        at += 3;
    } else {
        at += body.get(at..)?.iter().position(|b| *b == 0)? + 1;
    }
    let kind = *body.get(at)?;
    at += 1;
    let rest = body.get(at..)?;
    let skip = if encoding == 1 || encoding == 2 {
        // UTF-16: ends with two zero bytes on a character boundary.
        (0..rest.len().saturating_sub(1))
            .step_by(2)
            .find(|i| rest[*i] == 0 && rest[*i + 1] == 0)?
            + 2
    } else {
        rest.iter().position(|b| *b == 0)? + 1
    };
    Some((kind, rest.get(skip..)?))
}

fn apic_frame(picture: &Picture, major: u8) -> Vec<u8> {
    let mut body = vec![0u8]; // Latin-1 text
    body.extend_from_slice(picture.mime.as_bytes());
    body.push(0);
    body.push(3); // front cover
    body.push(0); // no description
    body.extend_from_slice(&picture.data);
    let mut frame = b"APIC".to_vec();
    let size = body.len() as u32;
    frame.extend_from_slice(&if major == 4 {
        to_syncsafe(size)
    } else {
        size.to_be_bytes()
    });
    frame.extend_from_slice(&[0, 0]);
    frame.extend_from_slice(&body);
    frame
}

/// `tag` (or a new ID3v2.3 tag) with `picture` added as the front cover,
/// using the tag's padding when there is room so the audio need not move.
/// `None` when it already has a picture or is a kind of tag this does not
/// rewrite (v2.2, unsynchronised, extended header, footer).
pub fn id3_with_picture(tag: Option<&[u8]>, picture: &Picture) -> Option<Vec<u8>> {
    let Some(tag) = tag else {
        let frame = apic_frame(picture, 3);
        let mut out = b"ID3\x03\x00\x00".to_vec();
        out.extend_from_slice(&to_syncsafe(frame.len() as u32));
        out.extend_from_slice(&frame);
        return Some(out);
    };
    let (major, start) = id3_layout(tag)?;
    if major < 3 || tag[5] & 0xD0 != 0 || start != 10 {
        return None;
    }
    let size = syncsafe(&tag[6..10]) as usize;
    let body = tag.get(10..10 + size)?;
    let frames = id3_frames(body, major);
    if frames.iter().any(|f| f.id == b"APIC") {
        return None;
    }
    let used: usize = frames.iter().map(|f| 10 + f.body.len()).sum();
    let padding = size - used;
    let frame = apic_frame(picture, major);
    let new_padding = padding.saturating_sub(frame.len());
    let new_size = frame.len() + used + new_padding;
    if new_size >= 1 << 28 {
        return None;
    }
    let mut out = tag[..6].to_vec();
    out.extend_from_slice(&to_syncsafe(new_size as u32));
    out.extend_from_slice(&frame);
    out.extend_from_slice(&body[..used]);
    out.resize(10 + new_size, 0);
    Some(out)
}

// ---- FLAC ----

/// The metadata blocks of a FLAC file as (type, body), and how many bytes
/// they take after the "fLaC" marker.
fn read_flac_head(file: &mut File) -> Option<(Vec<(u8, Vec<u8>)>, u64)> {
    let mut marker = [0u8; 4];
    file.read_exact(&mut marker).ok()?;
    if &marker != b"fLaC" {
        return None;
    }
    let mut blocks = Vec::new();
    let mut total = 0u64;
    loop {
        let mut header = [0u8; 4];
        file.read_exact(&mut header).ok()?;
        let len = (header[1] as usize) << 16 | (header[2] as usize) << 8 | header[3] as usize;
        total += 4 + len as u64;
        if total > MAX_HEADER || blocks.len() > 1000 {
            return None;
        }
        let mut body = vec![0u8; len];
        file.read_exact(&mut body).ok()?;
        blocks.push((header[0] & 0x7F, body));
        if header[0] & 0x80 != 0 {
            return Some((blocks, total));
        }
    }
}

const FLAC_PICTURE: u8 = 6;
const FLAC_PADDING: u8 = 1;

/// The front cover among FLAC metadata blocks (or else the first picture).
pub fn flac_picture(blocks: &[(u8, Vec<u8>)]) -> Option<Picture> {
    let mut first = None;
    for (kind, body) in blocks {
        if *kind != FLAC_PICTURE {
            continue;
        }
        let Some((role, image)) = flac_picture_image(body) else {
            continue;
        };
        let Some(picture) = picture(image) else {
            continue;
        };
        if role == 3 {
            return Some(picture);
        }
        first.get_or_insert(picture);
    }
    first
}

fn flac_picture_image(body: &[u8]) -> Option<(u32, &[u8])> {
    let role = be32(body.get(0..4)?);
    let mime_len = be32(body.get(4..8)?) as usize;
    let mut at = 8 + mime_len;
    let desc_len = be32(body.get(at..at + 4)?) as usize;
    at += 4 + desc_len + 16;
    let len = be32(body.get(at..at + 4)?) as usize;
    at += 4;
    Some((role, body.get(at..at + len)?))
}

/// Width and height of a PNG or baseline/progressive JPEG, for the FLAC
/// picture block (0 when they cannot be found; players ignore them).
fn image_size(data: &[u8]) -> (u32, u32) {
    if data.starts_with(&[0x89, b'P', b'N', b'G']) && data.len() >= 24 {
        return (be32(&data[16..20]), be32(&data[20..24]));
    }
    let mut at = 2;
    while at + 9 < data.len() {
        if data[at] != 0xFF {
            break;
        }
        let marker = data[at + 1];
        let len = (data[at + 2] as usize) << 8 | data[at + 3] as usize;
        if (0xC0..=0xCF).contains(&marker) && ![0xC4, 0xC8, 0xCC].contains(&marker) {
            let h = (data[at + 5] as u32) << 8 | data[at + 6] as u32;
            let w = (data[at + 7] as u32) << 8 | data[at + 8] as u32;
            return (w, h);
        }
        at += 2 + len;
    }
    (0, 0)
}

fn flac_picture_block(picture: &Picture) -> Vec<u8> {
    let (w, h) = image_size(&picture.data);
    let mut body = Vec::with_capacity(picture.data.len() + 64);
    body.extend_from_slice(&3u32.to_be_bytes());
    body.extend_from_slice(&(picture.mime.len() as u32).to_be_bytes());
    body.extend_from_slice(picture.mime.as_bytes());
    body.extend_from_slice(&0u32.to_be_bytes());
    body.extend_from_slice(&w.to_be_bytes());
    body.extend_from_slice(&h.to_be_bytes());
    body.extend_from_slice(&24u32.to_be_bytes());
    body.extend_from_slice(&0u32.to_be_bytes());
    body.extend_from_slice(&(picture.data.len() as u32).to_be_bytes());
    body.extend_from_slice(&picture.data);
    body
}

/// The metadata blocks (everything after "fLaC") with `picture` added
/// after the stream info, taking the room from a padding block when it is
/// big enough so the audio need not move. `None` if there is a picture
/// already.
pub fn flac_with_picture(blocks: &[(u8, Vec<u8>)], picture: &Picture) -> Option<Vec<u8>> {
    if blocks.is_empty()
        || blocks.iter().any(|(k, _)| *k == FLAC_PICTURE)
        || picture.data.len() > MAX_PICTURE
    {
        return None;
    }
    let new = flac_picture_block(picture);
    let needed = new.len() + 4;
    let mut list: Vec<(u8, Vec<u8>)> = Vec::with_capacity(blocks.len() + 1);
    list.push(blocks[0].clone());
    list.push((FLAC_PICTURE, new));
    let mut shrunk = false;
    for (kind, body) in &blocks[1..] {
        if !shrunk && *kind == FLAC_PADDING && body.len() >= needed {
            shrunk = true;
            list.push((FLAC_PADDING, vec![0; body.len() - needed]));
        } else {
            list.push((*kind, body.clone()));
        }
    }
    let mut out = Vec::new();
    let last = list.len() - 1;
    for (i, (kind, body)) in list.iter().enumerate() {
        let len = body.len();
        out.push(kind | if i == last { 0x80 } else { 0 });
        out.extend_from_slice(&[(len >> 16) as u8, (len >> 8) as u8, len as u8]);
        out.extend_from_slice(body);
    }
    Some(out)
}

// ---- MP4 (M4A, M4B, MOV, 3GP) ----

/// A box inside a buffer: where it starts, where its children (or data)
/// start, and where it ends.
#[derive(Clone, Copy, Debug)]
struct Atom {
    start: usize,
    body: usize,
    end: usize,
}

/// The boxes in `buf[from..to]`, with their types.
fn children(buf: &[u8], from: usize, to: usize) -> Vec<(&[u8], Atom)> {
    let mut out = Vec::new();
    let mut at = from;
    while at + 8 <= to {
        let size = be32(&buf[at..]) as usize;
        let kind = &buf[at + 4..at + 8];
        let (body, end) = match size {
            0 => (at + 8, to),
            1 => {
                let Some(big) = buf.get(at + 8..at + 16) else {
                    break;
                };
                let big = u64::from_be_bytes(big.try_into().unwrap_or_default());
                (at + 16, at.saturating_add(big as usize))
            }
            n if n < 8 => break,
            n => (at + 8, at + n),
        };
        if end > to {
            break;
        }
        out.push((
            kind,
            Atom {
                start: at,
                body,
                end,
            },
        ));
        at = end;
    }
    out
}

fn child(buf: &[u8], parent: Atom, kind: &[u8]) -> Option<Atom> {
    children(buf, parent.body, parent.end)
        .into_iter()
        .find(|(k, _)| *k == kind)
        .map(|(_, a)| a)
}

/// Where a `meta` box's children start: ISO files give it a version and
/// flags first, QuickTime files do not.
fn meta_children(buf: &[u8], meta: Atom) -> usize {
    let quicktime = buf
        .get(meta.body + 4..meta.body + 8)
        .is_some_and(|k| [&b"hdlr"[..], b"ilst", b"keys"].contains(&k));
    if quicktime { meta.body } else { meta.body + 4 }
}

/// The `moov` box and where it starts in the file.
fn read_moov(file: &mut File) -> Option<(Vec<u8>, u64)> {
    let len = file.metadata().ok()?.len();
    let mut at = 0u64;
    while at + 8 <= len {
        file.seek(SeekFrom::Start(at)).ok()?;
        let mut header = [0u8; 16];
        file.read_exact(&mut header[..8]).ok()?;
        let size = match be32(&header) as u64 {
            0 => len - at,
            1 => {
                file.read_exact(&mut header[8..]).ok()?;
                u64::from_be_bytes(header[8..16].try_into().ok()?)
            }
            n => n,
        };
        if size < 8 {
            return None;
        }
        if &header[4..8] == b"moov" {
            if size > MAX_HEADER || be32(&header) == 1 {
                return None;
            }
            let mut moov = vec![0u8; size as usize];
            file.seek(SeekFrom::Start(at)).ok()?;
            file.read_exact(&mut moov).ok()?;
            return Some((moov, at));
        }
        at += size;
    }
    None
}

/// Whether the media data comes after `moov` (at `moov_at`), so growing
/// `moov` moves it and the sample offsets must follow.
fn mdat_after(file: &mut File, moov_at: u64) -> Option<bool> {
    let len = file.metadata().ok()?.len();
    let mut at = 0u64;
    while at + 8 <= len {
        file.seek(SeekFrom::Start(at)).ok()?;
        let mut header = [0u8; 16];
        file.read_exact(&mut header[..8]).ok()?;
        let size = match be32(&header) as u64 {
            0 => len - at,
            1 => {
                file.read_exact(&mut header[8..]).ok()?;
                u64::from_be_bytes(header[8..16].try_into().ok()?)
            }
            n => n,
        };
        if size < 8 {
            return None;
        }
        if &header[4..8] == b"mdat" && at > moov_at {
            return Some(true);
        }
        at += size;
    }
    Some(false)
}

/// The cover in an MP4 `moov` box.
pub fn mp4_picture(moov: &[u8]) -> Option<Picture> {
    let root = Atom {
        start: 0,
        body: 8,
        end: moov.len(),
    };
    let udta = child(moov, root, b"udta")?;
    let meta = child(moov, udta, b"meta")?;
    let ilst = children(moov, meta_children(moov, meta), meta.end)
        .into_iter()
        .find(|(k, _)| *k == b"ilst")?
        .1;
    let covr = child(moov, ilst, b"covr")?;
    children(moov, covr.body, covr.end)
        .into_iter()
        .filter(|(k, _)| *k == b"data")
        .find_map(|(_, data)| picture(moov.get(data.body + 8..data.end)?))
}

fn atom(kind: &[u8], body: &[u8]) -> Vec<u8> {
    let mut out = ((body.len() + 8) as u32).to_be_bytes().to_vec();
    out.extend_from_slice(kind);
    out.extend_from_slice(body);
    out
}

/// `moov` with `picture` added as the cover (`covr` in udta/meta/ilst,
/// creating those as needed). `media_after`: the media data follows
/// `moov`, so its chunk offsets move by the growth. `None` when there is a
/// cover already or the box cannot be rewritten safely.
pub fn mp4_with_picture(moov: &[u8], picture: &Picture, media_after: bool) -> Option<Vec<u8>> {
    if moov.len() < 8 || &moov[4..8] != b"moov" || be32(moov) as usize != moov.len() {
        return None;
    }
    let root = Atom {
        start: 0,
        body: 8,
        end: moov.len(),
    };
    let kind = if picture.mime == "image/png" {
        14u32
    } else {
        13
    };
    let mut data = kind.to_be_bytes().to_vec();
    data.extend_from_slice(&[0; 4]);
    data.extend_from_slice(&picture.data);
    let covr = atom(b"covr", &atom(b"data", &data));
    let ilst = |items: &[u8]| atom(b"ilst", items);
    let hdlr = {
        let mut body = vec![0u8; 8];
        body.extend_from_slice(b"mdirappl");
        body.extend_from_slice(&[0; 9]);
        atom(b"hdlr", &body)
    };
    let meta = |inner: &[u8]| {
        let mut body = vec![0u8; 4];
        body.extend_from_slice(&hdlr);
        body.extend_from_slice(inner);
        atom(b"meta", &body)
    };
    // Where to insert, which boxes contain the insertion (their sizes grow)
    // and what to insert.
    let (at, parents, insert) = match child(moov, root, b"udta") {
        None => (root.end, vec![root], atom(b"udta", &meta(&ilst(&covr)))),
        Some(udta) => match child(moov, udta, b"meta") {
            None => (udta.end, vec![root, udta], meta(&ilst(&covr))),
            Some(m) => {
                let list = children(moov, meta_children(moov, m), m.end)
                    .into_iter()
                    .find(|(k, _)| *k == b"ilst")
                    .map(|(_, a)| a);
                match list {
                    None => (m.end, vec![root, udta, m], ilst(&covr)),
                    Some(list) => {
                        if child(moov, list, b"covr").is_some() {
                            return None;
                        }
                        (list.end, vec![root, udta, m, list], covr)
                    }
                }
            }
        },
    };
    let grow = insert.len();
    let mut out = Vec::with_capacity(moov.len() + grow);
    out.extend_from_slice(&moov[..at]);
    out.extend_from_slice(&insert);
    out.extend_from_slice(&moov[at..]);
    for parent in parents {
        if be32(&moov[parent.start..]) as usize != parent.end - parent.start {
            return None; // 64-bit or open-ended size
        }
        let size = (parent.end - parent.start + grow) as u32;
        out[parent.start..parent.start + 4].copy_from_slice(&size.to_be_bytes());
    }
    if media_after && !shift_chunk_offsets(&mut out, grow as u64) {
        return None;
    }
    Some(out)
}

/// Adds `by` to every chunk offset (stco and co64) in a `moov` box.
fn shift_chunk_offsets(moov: &mut [u8], by: u64) -> bool {
    let root = Atom {
        start: 0,
        body: 8,
        end: moov.len(),
    };
    let mut tables = Vec::new();
    for (kind, trak) in children(moov, root.body, root.end) {
        if kind != b"trak" {
            continue;
        }
        let path = [&b"mdia"[..], b"minf", b"stbl"];
        let mut at = Some(trak);
        for step in path {
            at = at.and_then(|a| child(moov, a, step));
        }
        let Some(stbl) = at else { continue };
        for (kind, table) in children(moov, stbl.body, stbl.end) {
            if kind == b"stco" || kind == b"co64" {
                tables.push((kind == b"co64", table));
            }
        }
    }
    for (wide, table) in tables {
        let Some(count) = moov.get(table.body + 4..table.body + 8).map(be32) else {
            return false;
        };
        let width = if wide { 8 } else { 4 };
        let first = table.body + 8;
        if first + count as usize * width > table.end {
            return false;
        }
        for i in 0..count as usize {
            let p = first + i * width;
            if wide {
                let v = u64::from_be_bytes(moov[p..p + 8].try_into().unwrap_or_default());
                moov[p..p + 8].copy_from_slice(&(v + by).to_be_bytes());
            } else {
                let Some(v) = (be32(&moov[p..]) as u64)
                    .checked_add(by)
                    .filter(|v| *v <= u32::MAX as u64)
                else {
                    return false;
                };
                moov[p..p + 4].copy_from_slice(&(v as u32).to_be_bytes());
            }
        }
    }
    true
}

#[cfg(test)]
pub mod tests {
    use super::*;

    /// A real 1x1 PNG.
    pub const PNG: [u8; 67] = [
        0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0x00, 0x00, 0x00, 0x0D, 0x49, 0x48, 0x44,
        0x52, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x06, 0x00, 0x00, 0x00, 0x1F,
        0x15, 0xC4, 0x89, 0x00, 0x00, 0x00, 0x0A, 0x49, 0x44, 0x41, 0x54, 0x78, 0x9C, 0x63, 0x00,
        0x01, 0x00, 0x00, 0x05, 0x00, 0x01, 0x0D, 0x0A, 0x2D, 0xB4, 0x00, 0x00, 0x00, 0x00, 0x49,
        0x45, 0x4E, 0x44, 0xAE, 0x42, 0x60, 0x82,
    ];

    pub fn png() -> Picture {
        Picture {
            mime: "image/png",
            data: PNG.to_vec(),
        }
    }

    fn jpeg() -> Picture {
        // SOI, an SOF0 segment for 300 x 200, EOI: enough to recognise.
        let data = vec![
            0xFF, 0xD8, 0xFF, 0xC0, 0x00, 0x11, 0x08, 0x00, 0xC8, 0x01, 0x2C, 0x03, 0x01, 0x22,
            0x00, 0x02, 0x11, 0x01, 0x03, 0x11, 0x01, 0xFF, 0xD9,
        ];
        Picture {
            mime: "image/jpeg",
            data,
        }
    }

    #[test]
    fn recognises_jpeg_and_png_only() {
        assert_eq!(image_mime(&PNG), Some("image/png"));
        assert_eq!(image_mime(&jpeg().data), Some("image/jpeg"));
        assert_eq!(image_mime(b"GIF89a"), None);
        assert_eq!(image_size(&PNG), (1, 1));
        assert_eq!(image_size(&jpeg().data), (300, 200));
    }

    #[test]
    fn id3_round_trip_uses_padding() {
        // A new tag.
        let tag = id3_with_picture(None, &png()).unwrap();
        assert_eq!(id3_picture(&tag), Some(png()));
        // An existing v2.3 tag with a title and 2000 bytes of padding.
        let mut frames = b"TIT2".to_vec();
        frames.extend_from_slice(&6u32.to_be_bytes());
        frames.extend_from_slice(&[0, 0, 0]);
        frames.extend_from_slice(b"Hello");
        let size = frames.len() + 2000;
        let mut old = b"ID3\x03\x00\x00".to_vec();
        old.extend_from_slice(&to_syncsafe(size as u32));
        old.extend_from_slice(&frames);
        old.resize(10 + size, 0);
        let new = id3_with_picture(Some(&old), &jpeg()).unwrap();
        assert_eq!(new.len(), old.len(), "fits in the padding");
        assert_eq!(id3_picture(&new), Some(jpeg()));
        let (major, start) = id3_layout(&new).unwrap();
        let ids: Vec<_> = id3_frames(&new[start..], major)
            .iter()
            .map(|f| f.id.to_vec())
            .collect();
        assert_eq!(ids, vec![b"APIC".to_vec(), b"TIT2".to_vec()]);
        // Never a second picture.
        assert_eq!(id3_with_picture(Some(&new), &png()), None);
        // v2.4 sizes are syncsafe; no padding means the tag grows.
        let mut v4 = b"ID3\x04\x00\x00".to_vec();
        v4.extend_from_slice(&to_syncsafe(frames.len() as u32));
        let mut f4 = b"TIT2".to_vec();
        f4.extend_from_slice(&to_syncsafe(6));
        f4.extend_from_slice(&[0, 0, 0]);
        f4.extend_from_slice(b"Hello");
        v4.extend_from_slice(&f4);
        let new4 = id3_with_picture(Some(&v4), &png()).unwrap();
        assert!(new4.len() > v4.len());
        assert_eq!(id3_picture(&new4), Some(png()));
    }

    #[test]
    fn id3_reads_utf16_descriptions_and_v22() {
        let mut body = vec![1u8];
        body.extend_from_slice(b"image/png\0");
        body.push(3);
        body.extend_from_slice(&[0xFF, 0xFE, b'A', 0, 0, 0]);
        body.extend_from_slice(&PNG);
        assert_eq!(apic_image(&body, false), Some((3, &PNG[..])));
        let mut v22 = vec![0u8];
        v22.extend_from_slice(b"PNG");
        v22.push(0);
        v22.extend_from_slice(b"cover\0");
        v22.extend_from_slice(&PNG);
        assert_eq!(apic_image(&v22, true), Some((0, &PNG[..])));
    }

    #[test]
    fn flac_round_trip() {
        let streaminfo = (0u8, vec![0u8; 34]);
        let comment = (4u8, b"comments".to_vec());
        let padding = (FLAC_PADDING, vec![0u8; 4096]);
        let blocks = vec![streaminfo.clone(), comment.clone(), padding];
        let head = flac_with_picture(&blocks, &png()).unwrap();
        let old_len: usize = blocks.iter().map(|(_, b)| 4 + b.len()).sum();
        assert_eq!(head.len(), old_len, "took the room from the padding");
        // Parse it back.
        let mut file = b"fLaC".to_vec();
        file.extend_from_slice(&head);
        let path = std::env::temp_dir().join(format!("mac-art-{}.flac", std::process::id()));
        std::fs::write(&path, &file).unwrap();
        let (back, len) = read_flac_head(&mut File::open(&path).unwrap()).unwrap();
        assert_eq!(len as usize, head.len());
        assert_eq!(back[0], streaminfo);
        assert_eq!(back[1].0, FLAC_PICTURE);
        assert_eq!(back[2], comment);
        assert_eq!(flac_picture(&back), Some(png()));
        assert_eq!(flac_with_picture(&back, &png()), None);
        // Without padding it grows, and only the last block is marked last.
        let grown = flac_with_picture(&[streaminfo], &png()).unwrap();
        assert_eq!(grown[0] & 0x80, 0);
        let _ = std::fs::remove_file(&path);
    }

    fn moov_with(children: &[Vec<u8>]) -> Vec<u8> {
        atom(b"moov", &children.concat())
    }

    fn trak_with_offsets(offsets: &[u32]) -> Vec<u8> {
        let mut stco = vec![0u8; 4];
        stco.extend_from_slice(&(offsets.len() as u32).to_be_bytes());
        for o in offsets {
            stco.extend_from_slice(&o.to_be_bytes());
        }
        let stbl = atom(b"stbl", &atom(b"stco", &stco));
        atom(b"trak", &atom(b"mdia", &atom(b"minf", &stbl)))
    }

    fn offsets(moov: &[u8]) -> Vec<u32> {
        let root = Atom {
            start: 0,
            body: 8,
            end: moov.len(),
        };
        let trak = child(moov, root, b"trak").unwrap();
        let stbl = [&b"mdia"[..], b"minf", b"stbl"]
            .iter()
            .fold(trak, |a, k| child(moov, a, k).unwrap());
        let stco = child(moov, stbl, b"stco").unwrap();
        let n = be32(&moov[stco.body + 4..]) as usize;
        (0..n)
            .map(|i| be32(&moov[stco.body + 8 + i * 4..]))
            .collect()
    }

    #[test]
    fn mp4_adds_cover_at_every_depth_and_moves_offsets() {
        let mvhd = atom(b"mvhd", &[0; 100]);
        let trak = trak_with_offsets(&[1000, 5000]);
        // No udta at all.
        let bare = moov_with(&[mvhd.clone(), trak.clone()]);
        let new = mp4_with_picture(&bare, &png(), false).unwrap();
        assert_eq!(mp4_picture(&new), Some(png()));
        assert_eq!(be32(&new) as usize, new.len());
        assert_eq!(offsets(&new), vec![1000, 5000]);
        // Media after moov: offsets move by the growth.
        let moved = mp4_with_picture(&bare, &png(), true).unwrap();
        let grow = (moved.len() - bare.len()) as u32;
        assert_eq!(offsets(&moved), vec![1000 + grow, 5000 + grow]);
        // An existing ilst with a title, as Windows writes it.
        let title = atom(b"\xA9nam", &atom(b"data", b"\0\0\0\x01\0\0\0\0Hi"));
        let mut meta_body = vec![0u8; 4];
        meta_body.extend_from_slice(&atom(b"hdlr", &[0; 25]));
        meta_body.extend_from_slice(&atom(b"ilst", &title));
        let udta = atom(b"udta", &atom(b"meta", &meta_body));
        let tagged = moov_with(&[mvhd, udta, trak]);
        let new = mp4_with_picture(&tagged, &jpeg(), false).unwrap();
        assert_eq!(mp4_picture(&new), Some(jpeg()));
        // covr box, data box, type and locale, then the picture.
        assert_eq!(new.len(), tagged.len() + 8 + 8 + 8 + jpeg().data.len());
        assert_eq!(mp4_with_picture(&new, &png(), false), None);
    }
}
