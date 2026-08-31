//! Where a cover comes from.
//!
//! Murk never downloads artwork: a search by title is a metadata leak, and a
//! promotional poster routinely shows a scene from the finale. A cover is
//! either a file the user already has in the series folder, or a picture
//! embedded in one of the episode files, or a file the user picked by hand.
//! All three are local, so nothing leaves the machine.

use std::collections::HashMap;
use std::fs::{self, File};
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

/// How big an extracted picture may be. Anything larger is not a cover.
const MAX_PAYLOAD: u64 = 16 * 1024 * 1024;
/// Largest poster served to the frontend as a data URL (and accepted from a
/// user's pick).
pub const MAX_DATA_URL: usize = 20 * 1024 * 1024;

/// Conventional cover filenames inside a series folder, best first. These are
/// the names Kodi, Plex and a plain "put a jpg next to the files" workflow all
/// use.
const LOCAL_POSTER_NAMES: &[&str] = &[
    "poster.jpg",
    "poster.jpeg",
    "poster.png",
    "poster.webp",
    "cover.jpg",
    "cover.jpeg",
    "cover.png",
    "folder.jpg",
    "folder.png",
    "default.jpg",
];

/// Find a cover file the user placed in the series folder.
pub fn local_poster(root: &Path) -> Option<PathBuf> {
    let mut by_name = std::collections::HashMap::new();
    for entry in fs::read_dir(root).ok()? {
        let entry = entry.ok()?;
        if !entry.file_type().ok()?.is_file() {
            continue;
        }
        let name = entry.file_name().to_string_lossy().to_lowercase();
        by_name.entry(name).or_insert(entry.path());
    }
    LOCAL_POSTER_NAMES
        .iter()
        .find_map(|candidate| by_name.get(*candidate).cloned())
}

// --- embedded artwork ------------------------------------------------------
//
// Two containers carry cover art: Matroska keeps it as an attachment in the
// `Attachments` element, MP4/MOV keeps it as a `covr` atom. Both parsers read
// only as much of the file as they need and give up gracefully on anything
// they do not understand: a missing cover is `None`, never an error.

// The vint *values* of the element ids: the marker bit that encodes the id
// length is stripped when decoding, so a Segment (bytes `18 53 80 67`) decodes
// to 0x0853_8067.
const EBML_SEGMENT: u64 = 0x0853_8067;
const EBML_ATTACHMENTS: u64 = 0x0941_A469;
const EBML_ATTACHED_FILE: u64 = 0x21A7;
const EBML_FILE_MIME: u64 = 0x0660;
const EBML_FILE_DATA: u64 = 0x065C;

/// Extract a cover embedded in an episode file (MKV or MP4), writing it into
/// `covers_dir` as `{series_id}.{ext}`. Returns the written path.
pub fn extract_embedded(episode: &Path, covers_dir: &Path, series_id: i64) -> Option<PathBuf> {
    let (bytes, ext) = embedded_cover(episode)?;
    clear_cached(covers_dir, series_id);
    let dest = covers_dir.join(format!("{series_id}.{ext}"));
    fs::write(&dest, bytes).ok()?;
    Some(dest)
}

/// The raw picture and its file extension, if the container has one.
pub fn embedded_cover(path: &Path) -> Option<(Vec<u8>, &'static str)> {
    let mut f = File::open(path).ok()?;
    if let Some(found) = mkv_cover(&mut f) {
        return Some(found);
    }
    let mut f = File::open(path).ok()?;
    mp4_cover(&mut f)
}

/// Remove any previously cached cover for the series, whatever its extension.
fn clear_cached(covers_dir: &Path, series_id: i64) {
    if let Ok(entries) = fs::read_dir(covers_dir) {
        for entry in entries.flatten() {
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if name.starts_with(&format!("{series_id}.")) {
                let _ = fs::remove_file(entry.path());
            }
        }
    }
}

// --- Matroska / EBML -------------------------------------------------------

/// A cursor over a file with a byte budget for the enclosing container.
///
/// `remaining` is how many bytes of the current element are left to read; the
/// top-level walk has no budget (`None`) and stops at EOF.
struct EbmlCursor<'a> {
    file: &'a mut File,
    remaining: Option<u64>,
}

impl EbmlCursor<'_> {
    /// Consume `n` bytes from the budget, if the container allows it.
    fn charge(&mut self, n: u64) -> Option<()> {
        if let Some(rem) = self.remaining.as_mut() {
            if *rem < n {
                return None;
            }
            *rem -= n;
        }
        Some(())
    }

    /// A variable-length integer (EBML ID or element size).
    fn vint(&mut self) -> Option<(u64, u8)> {
        let mut first = [0u8; 1];
        self.file.read_exact(&mut first).ok()?;
        self.charge(1)?;

        let mut marker = 0x80u8;
        let mut len = 1u8;
        while first[0] & marker == 0 && len < 8 {
            marker >>= 1;
            len += 1;
        }

        let mut value = (first[0] & (marker - 1)) as u64;
        for _ in 1..len {
            let mut byte = [0u8; 1];
            self.file.read_exact(&mut byte).ok()?;
            self.charge(1)?;
            value = (value << 8) | byte[0] as u64;
        }
        Some((value, len))
    }

    /// Jump over `n` bytes without reading them (a Cluster is gigabytes).
    fn skip(&mut self, n: u64) -> Option<()> {
        self.charge(n)?;
        let pos = self.file.stream_position().ok()?;
        self.file.seek(SeekFrom::Start(pos + n)).ok()?;
        Some(())
    }

    /// Read `n` bytes as data.
    fn read_bytes(&mut self, n: u64) -> Option<Vec<u8>> {
        if n > MAX_PAYLOAD {
            return None;
        }
        self.charge(n)?;
        let mut buf = vec![0u8; n as usize];
        self.file.read_exact(&mut buf).ok()?;
        Some(buf)
    }
}

fn mkv_cover(f: &mut File) -> Option<(Vec<u8>, &'static str)> {
    walk_ebml(&mut EbmlCursor {
        file: f,
        remaining: None,
    })
}

fn walk_ebml(c: &mut EbmlCursor<'_>) -> Option<(Vec<u8>, &'static str)> {
    loop {
        if c.remaining == Some(0) {
            return None;
        }
        let (id, _) = c.vint()?;
        let (size, len) = c.vint()?;
        let unknown = size == (1u64 << (7 * len as u64)) - 1;

        match id {
            EBML_SEGMENT => {
                let saved = c.remaining;
                c.remaining = if unknown { None } else { Some(size) };
                let found = walk_ebml(c);
                c.remaining = saved;
                if found.is_some() {
                    return found;
                }
            }
            EBML_ATTACHMENTS if !unknown => return parse_attachments(c, size),
            _ => {
                if unknown {
                    // An unbounded element we do not know how to skip.
                    return None;
                }
                c.skip(size)?;
            }
        }
    }
}

fn parse_attachments(c: &mut EbmlCursor<'_>, budget: u64) -> Option<(Vec<u8>, &'static str)> {
    let saved = c.remaining;
    c.remaining = Some(budget);

    let mut result = None;
    loop {
        if c.remaining == Some(0) {
            break;
        }
        let Some((id, _)) = c.vint() else { break };
        let Some((size, len)) = c.vint() else { break };
        let unknown = size == (1u64 << (7 * len as u64)) - 1;
        if id == EBML_ATTACHED_FILE {
            if unknown {
                break;
            }
            if let Some(found) = parse_attached_file(c, size) {
                result = Some(found);
                break;
            }
        } else if unknown {
            break;
        } else {
            c.skip(size)?;
        }
    }

    c.remaining = saved;
    result
}

fn parse_attached_file(c: &mut EbmlCursor<'_>, budget: u64) -> Option<(Vec<u8>, &'static str)> {
    let saved = c.remaining;
    c.remaining = Some(budget);

    let mut mime: Option<String> = None;
    let mut data: Option<Vec<u8>> = None;
    while c.remaining != Some(0) {
        let Some((id, _)) = c.vint() else { break };
        let Some((size, len)) = c.vint() else { break };
        let unknown = size == (1u64 << (7 * len as u64)) - 1;
        if unknown {
            break;
        }
        match id {
            EBML_FILE_MIME => {
                mime = Some(String::from_utf8_lossy(&c.read_bytes(size)?).into_owned());
            }
            EBML_FILE_DATA => {
                data = c.read_bytes(size);
            }
            _ => {
                c.skip(size)?;
            }
        }
    }

    c.remaining = saved;
    let mime = mime?;
    let data = data?;
    match mime.as_str() {
        "image/jpeg" => Some((data, "jpg")),
        "image/png" => Some((data, "png")),
        _ => None,
    }
}

// --- MP4 / ISO-BMFF --------------------------------------------------------

/// Read one atom header, returning its payload size (bytes after the header)
/// and its four-character type. `remaining` is the byte budget of the enclosing
/// container.
fn mp4_atom(f: &mut File, remaining: &mut Option<u64>) -> Option<(u64, [u8; 4])> {
    let mut header = [0u8; 8];
    f.read_exact(&mut header).ok()?;
    charge(remaining, 8)?;

    let size32 = u32::from_be_bytes(header[..4].try_into().ok()?);
    let ty: [u8; 4] = header[4..].try_into().ok()?;

    let payload: u64 = if size32 == 1 {
        let mut extended = [0u8; 8];
        f.read_exact(&mut extended).ok()?;
        charge(remaining, 8)?;
        u64::from_be_bytes(extended).checked_sub(16)?
    } else if size32 == 0 {
        // Runs to the end of the container.
        remaining.unwrap_or(u64::MAX)
    } else {
        size32 as u64 - 8
    };
    Some((payload, ty))
}

fn charge(remaining: &mut Option<u64>, n: u64) -> Option<()> {
    if let Some(rem) = remaining {
        if *rem < n {
            return None;
        }
        *rem -= n;
    }
    Some(())
}

fn mp4_skip(f: &mut File, remaining: &mut Option<u64>, n: u64) -> Option<()> {
    charge(remaining, n)?;
    let pos = f.stream_position().ok()?;
    f.seek(SeekFrom::Start(pos + n)).ok()?;
    Some(())
}

fn mp4_cover(f: &mut File) -> Option<(Vec<u8>, &'static str)> {
    // moov -> udta -> meta -> ilst -> covr -> data
    let mut remaining = None;
    loop {
        let (size, ty) = mp4_atom(f, &mut remaining)?;
        if ty == *b"moov" {
            let mut moov_rem = Some(size);
            loop {
                let (size, ty) = mp4_atom(f, &mut moov_rem)?;
                if ty == *b"udta" {
                    let mut udta_rem = Some(size);
                    loop {
                        let (size, ty) = mp4_atom(f, &mut udta_rem)?;
                        if ty == *b"meta" {
                            // `meta` is a FullBox: four bytes of version and
                            // flags precede its children.
                            mp4_skip(f, &mut udta_rem, 4)?;
                            let mut meta_rem = Some(size.checked_sub(4)?);
                            loop {
                                let (size, ty) = mp4_atom(f, &mut meta_rem)?;
                                if ty == *b"ilst" {
                                    // ilst → covr → data
                                    let mut ilst_rem = Some(size);
                                    loop {
                                        let (csize, cty) = mp4_atom(f, &mut ilst_rem)?;
                                        if cty == *b"covr" {
                                            return parse_covr(f, csize);
                                        }
                                        mp4_skip(f, &mut ilst_rem, csize)?;
                                    }
                                }
                                mp4_skip(f, &mut meta_rem, size)?;
                            }
                        }
                        mp4_skip(f, &mut udta_rem, size)?;
                    }
                }
                mp4_skip(f, &mut moov_rem, size)?;
            }
        }
        mp4_skip(f, &mut remaining, size)?;
    }
}

fn parse_covr(f: &mut File, budget: u64) -> Option<(Vec<u8>, &'static str)> {
    let mut remaining = Some(budget);
    while remaining != Some(0) {
        let (size, ty) = mp4_atom(f, &mut remaining)?;
        if ty != *b"data" {
            mp4_skip(f, &mut remaining, size)?;
            continue;
        }
        // A `data` FullBox: version/flags (4) + type indicator (4) + locale (4).
        let mut head = [0u8; 12];
        f.read_exact(&mut head).ok()?;
        charge(&mut remaining, 12)?;
        let kind = u32::from_be_bytes(head[4..8].try_into().ok()?);
        let ext = match kind {
            13 => "jpg", // JPEG
            14 => "png", // PNG
            _ => {
                mp4_skip(f, &mut remaining, size.checked_sub(12)?)?;
                continue;
            }
        };
        let n = size.checked_sub(12)?;
        if n > MAX_PAYLOAD {
            return None;
        }
        charge(&mut remaining, n)?;
        let mut bytes = vec![0u8; n as usize];
        f.read_exact(&mut bytes).ok()?;
        return Some((bytes, ext));
    }
    None
}

// --- serving ---------------------------------------------------------------

/// A poster as a `data:` URL the webview can put straight into an `<img>`.
///
/// A data URL keeps the frontend out of the filesystem entirely: there is no
/// asset-protocol scope to configure, no arbitrary path crossing the IPC
/// boundary. Posters are a few hundred KB, so this stays small.
pub fn data_url(path: &Path) -> Option<String> {
    let bytes = fs::read(path).ok()?;
    if bytes.is_empty() || bytes.len() > MAX_DATA_URL {
        return None;
    }
    let mime = mime_for(path)?;
    let encoded = base64::Engine::encode(&base64::engine::general_purpose::STANDARD, &bytes);
    Some(format!("data:{mime};base64,{encoded}"))
}

fn mime_for(path: &Path) -> Option<&'static str> {
    match path
        .extension()
        .and_then(|e| e.to_str())
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        Some("jpg") | Some("jpeg") => Some("image/jpeg"),
        Some("png") => Some("image/png"),
        Some("webp") => Some("image/webp"),
        _ => None,
    }
}

/// A cache of rendered data URLs, keyed by the cover file and invalidated when
/// its modification time or size changes.
///
/// Without this, every library refresh re-reads and re-encodes every poster on
/// the command thread: with several large covers, megabytes of base64 per
/// refresh. A file changed on disk still picks up on the next read.
pub struct DataUrlCache {
    entries: HashMap<PathBuf, Cached>,
}

struct Cached {
    modified: SystemTime,
    len: u64,
    url: Option<String>,
}

impl Default for DataUrlCache {
    fn default() -> Self {
        Self::new()
    }
}

impl DataUrlCache {
    pub fn new() -> Self {
        Self {
            entries: HashMap::new(),
        }
    }

    /// The data URL for `path`, reusing the previous render while the file is
    /// unchanged. A file that disappeared evicts its entry.
    pub fn get(&mut self, path: &Path) -> Option<String> {
        let meta = match fs::metadata(path) {
            Ok(meta) => meta,
            Err(_) => {
                self.entries.remove(path);
                return None;
            }
        };
        let modified = meta.modified().unwrap_or(SystemTime::UNIX_EPOCH);
        let len = meta.len();
        if let Some(cached) = self.entries.get(path) {
            if cached.modified == modified && cached.len == len {
                return cached.url.clone();
            }
        }
        let url = data_url(path);
        self.entries.insert(
            path.to_path_buf(),
            Cached {
                modified,
                len,
                url: url.clone(),
            },
        );
        url
    }
}

/// The file extensions a user-picked poster may have.
pub const ACCEPTED_POSTER_EXTENSIONS: &[&str] = &["jpg", "jpeg", "png", "webp"];

#[cfg(test)]
mod tests {
    use super::*;

    fn tempdir() -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "murk-poster-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn local_poster_finds_conventional_names() {
        let dir = tempdir();
        fs::write(dir.join("folder.jpg"), b"jpeg-bytes").unwrap();
        assert_eq!(
            local_poster(&dir),
            Some(dir.join("folder.jpg")),
            "folder.jpg is a conventional cover name"
        );
        assert!(local_poster(&dir.join("missing")).is_none());
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn local_poster_prefers_poster_over_cover() {
        let dir = tempdir();
        fs::write(dir.join("poster.png"), b"png").unwrap();
        fs::write(dir.join("folder.jpg"), b"jpeg").unwrap();
        assert_eq!(local_poster(&dir), Some(dir.join("poster.png")));
        fs::remove_dir_all(&dir).ok();
    }

    /// Build a minimal MKV: EBML header, a Segment containing one attachment
    /// with a JPEG payload.
    fn minimal_mkv(jpeg: &[u8]) -> Vec<u8> {
        fn vint(value: u64) -> Vec<u8> {
            let mut n = 1usize;
            while value >= (1u64 << (7 * n)) {
                n += 1;
            }
            let mut out = vec![0u8; n];
            out[0] = ((value >> (8 * (n - 1))) as u8) | (0x80u8 >> (n - 1));
            for (i, slot) in out.iter_mut().enumerate().skip(1) {
                *slot = ((value >> (8 * (n - 1 - i))) & 0xFF) as u8;
            }
            out
        }
        fn element(id: u64, payload: &[u8]) -> Vec<u8> {
            let mut out = vint(id);
            out.extend(vint(payload.len() as u64));
            out.extend(payload);
            out
        }
        fn attached_file(mime: &str, data: &[u8]) -> Vec<u8> {
            let mut body = Vec::new();
            body.extend(element(EBML_FILE_MIME, mime.as_bytes()));
            body.extend(element(EBML_FILE_DATA, data));
            element(EBML_ATTACHED_FILE, &body)
        }

        let mut attachments = Vec::new();
        attachments.extend(attached_file("image/jpeg", jpeg));
        attachments.extend(attached_file("image/png", b"not-a-png"));
        let attachments = element(EBML_ATTACHMENTS, &attachments);

        let mut segment = Vec::new();
        segment.extend(attachments);
        segment.extend(element(0x0F43_B675, &[0u8; 8])); // a Cluster
        let segment = element(EBML_SEGMENT, &segment);

        let mut header = element(0x0A45_DFA3, &[0u8; 8]); // the EBML header
        header.extend(segment);
        header
    }

    #[test]
    fn extracts_an_mkv_attachment() {
        let dir = tempdir();
        let file = dir.join("ep.mkv");
        let jpeg = vec![0xFF, 0xD8, 0xFF, 0xE0, 1, 2, 3];
        fs::write(&file, minimal_mkv(&jpeg)).unwrap();
        let (bytes, ext) = embedded_cover(&file).unwrap();
        assert_eq!(bytes, jpeg);
        assert_eq!(ext, "jpg");
        fs::remove_dir_all(&dir).ok();
    }

    /// Build a minimal MP4 with moov/udta/meta/ilst/covr/data holding a PNG.
    fn minimal_mp4(png: &[u8]) -> Vec<u8> {
        fn atom(ty: &[u8; 4], payload: &[u8]) -> Vec<u8> {
            let mut out = Vec::new();
            out.extend(((payload.len() + 8) as u32).to_be_bytes());
            out.extend(ty);
            out.extend(payload);
            out
        }
        // data fullbox: version/flags(4) + kind(4) + locale(4) + payload
        let mut data = Vec::new();
        data.extend([0u8; 4]);
        data.extend(14u32.to_be_bytes()); // PNG
        data.extend([0u8; 4]);
        data.extend(png);
        let data = atom(b"data", &data);

        let covr = atom(b"covr", &data);
        let ilst = atom(b"ilst", &covr);
        let meta = {
            let mut body = Vec::new();
            body.extend([0u8; 4]); // version/flags
            body.extend(ilst);
            atom(b"meta", &body)
        };
        let udta = atom(b"udta", &meta);
        let moov = atom(b"moov", &udta);
        let mut file = atom(b"ftyp", b"isom");
        file.extend(moov);
        file
    }

    #[test]
    fn extracts_an_mp4_covr() {
        let dir = tempdir();
        let file = dir.join("ep.mp4");
        let png = vec![0x89, 0x50, 0x4E, 0x47, 9, 8, 7];
        fs::write(&file, minimal_mp4(&png)).unwrap();
        let (bytes, ext) = embedded_cover(&file).unwrap();
        assert_eq!(bytes, png);
        assert_eq!(ext, "png");
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn no_cover_in_an_unrelated_file() {
        let dir = tempdir();
        let file = dir.join("notes.txt");
        fs::write(&file, b"not a container").unwrap();
        assert!(embedded_cover(&file).is_none());
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn data_url_carries_the_right_mime() {
        let dir = tempdir();
        let file = dir.join("poster.jpg");
        fs::write(&file, b"jpeg").unwrap();
        let url = data_url(&file).unwrap();
        assert!(url.starts_with("data:image/jpeg;base64,"));
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn data_url_cache_reuses_and_invalidates() {
        use std::io::Write;

        let dir = tempdir();
        let file = dir.join("poster.jpg");
        fs::write(&file, b"jpeg").unwrap();
        let mut cache = DataUrlCache::new();

        let first = cache.get(&file);
        let second = cache.get(&file);
        assert_eq!(first, second, "an unchanged file is served from the cache");
        assert!(first.is_some());

        // A different length evicts the entry even without a mtime change.
        fs::write(&file, b"a much longer poster").unwrap();
        let third = cache.get(&file);
        assert_ne!(third, first, "a resized file must be re-encoded");
        assert!(third.is_some());

        // Same length again, but a different mtime must also invalidate.
        let past = SystemTime::now()
            .checked_sub(std::time::Duration::from_secs(10))
            .unwrap();
        {
            let mut f = fs::OpenOptions::new().write(true).open(&file).unwrap();
            f.write_all(b"a much longer postee").unwrap();
            f.set_modified(past).unwrap();
        }
        let fourth = cache.get(&file);
        assert_ne!(fourth, third, "a rewritten file must be re-encoded");
        assert!(fourth.is_some());

        // A missing file evicts the entry and yields nothing.
        fs::remove_file(&file).unwrap();
        assert!(cache.get(&file).is_none());
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn cache_cleanup_drops_old_extensions() {
        let dir = tempdir();
        fs::create_dir_all(dir.join("covers")).unwrap();
        let covers = dir.join("covers");
        fs::write(covers.join("7.png"), b"old").unwrap();
        let dir2 = tempdir();
        let file = dir2.join("ep.mkv");
        let jpeg = vec![1, 2, 3, 4];
        fs::write(&file, minimal_mkv(&jpeg)).unwrap();
        let written = extract_embedded(&file, &covers, 7).unwrap();
        assert_eq!(written, covers.join("7.jpg"));
        assert!(
            !covers.join("7.png").exists(),
            "old extension must be removed"
        );
        fs::remove_dir_all(&dir).ok();
        fs::remove_dir_all(&dir2).ok();
    }
}
