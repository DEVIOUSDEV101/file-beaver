//! File formats File Beaver reads and writes.

use std::path::Path;

#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
pub enum Format {
    Jpg,
    Png,
    Webp,
    Heic,
    Tiff,
    Avif,
    Bmp,
    Gif,
    Pdf,
    Docx,
    /// Older and other word-processor formats Word opens; read only, through Word.
    Doc,
    Rtf,
    Odt,
}

impl Format {
    pub fn from_path(path: &Path) -> Option<Format> {
        let ext = path.extension()?.to_str()?.to_ascii_lowercase();
        Some(match ext.as_str() {
            "jpg" | "jpeg" | "jfif" => Format::Jpg,
            "png" => Format::Png,
            "webp" => Format::Webp,
            "heic" | "heif" => Format::Heic,
            "tif" | "tiff" => Format::Tiff,
            "avif" => Format::Avif,
            "bmp" => Format::Bmp,
            "gif" => Format::Gif,
            "pdf" => Format::Pdf,
            "docx" => Format::Docx,
            "doc" => Format::Doc,
            "rtf" => Format::Rtf,
            "odt" => Format::Odt,
            _ => return None,
        })
    }

    /// The id used in ring segment ids ("convert:png").
    pub fn from_id(id: &str) -> Option<Format> {
        Format::from_path(Path::new(&format!("x.{id}")))
    }

    pub fn id(self) -> &'static str {
        match self {
            Format::Jpg => "jpg",
            Format::Png => "png",
            Format::Webp => "webp",
            Format::Heic => "heic",
            Format::Tiff => "tiff",
            Format::Avif => "avif",
            Format::Bmp => "bmp",
            Format::Gif => "gif",
            Format::Pdf => "pdf",
            Format::Docx => "docx",
            Format::Doc => "doc",
            Format::Rtf => "rtf",
            Format::Odt => "odt",
        }
    }

    pub fn ext(self) -> &'static str {
        self.id()
    }

    pub fn label(self) -> String {
        self.id().to_ascii_uppercase()
    }

    pub fn is_image(self) -> bool {
        !matches!(self, Format::Pdf) && !self.is_word()
    }

    /// Documents Word opens (and File Beaver only reads through Word).
    pub fn is_word(self) -> bool {
        matches!(self, Format::Docx | Format::Doc | Format::Rtf | Format::Odt)
    }
}

/// The real format from the file's first bytes, whatever its name says
/// (downloads are often misnamed, e.g. a PNG saved as .jpg).
pub fn sniff(path: &Path) -> Option<Format> {
    use std::io::Read;
    let mut head = [0u8; 16];
    let n = std::fs::File::open(path).ok()?.read(&mut head).ok()?;
    let h = &head[..n];
    Some(if h.starts_with(&[0xFF, 0xD8, 0xFF]) {
        Format::Jpg
    } else if h.starts_with(b"\x89PNG") {
        Format::Png
    } else if h.len() >= 12 && &h[0..4] == b"RIFF" && &h[8..12] == b"WEBP" {
        Format::Webp
    } else if h.starts_with(b"GIF8") {
        Format::Gif
    } else if h.starts_with(b"BM") {
        Format::Bmp
    } else if h.starts_with(b"II*\0") || h.starts_with(b"MM\0*") {
        Format::Tiff
    } else if h.starts_with(b"%PDF") {
        Format::Pdf
    } else if h.len() >= 12 && &h[4..8] == b"ftyp" {
        match &h[8..12] {
            b"avif" | b"avis" => Format::Avif,
            b"heic" | b"heix" | b"heim" | b"heis" | b"mif1" | b"msf1" => Format::Heic,
            _ => return None,
        }
    } else {
        return None;
    })
}

/// A file whose name says it's an image but whose content isn't recognisable.
pub fn not_really(path: &std::path::Path) -> Option<String> {
    let named = Format::from_path(path)?;
    let ok = if named.is_word() { looks_like_word(path) } else { named == Format::Pdf || sniff(path).is_some() };
    (!ok).then(|| format!("this file is damaged or isn't really a {}", named.label()))
}

/// A zip (DOCX, ODT), an old Word binary file (DOC) or RTF. Word itself decides the rest;
/// this only catches files that can't be a document at all (empty, an image renamed .docx).
pub fn looks_like_word(path: &Path) -> bool {
    use std::io::Read;
    let mut head = [0u8; 8];
    let Ok(n) = std::fs::File::open(path).and_then(|mut f| f.read(&mut head)) else { return false };
    let h = &head[..n];
    h.starts_with(&[b'P', b'K', 3, 4]) || h.starts_with(&[0xD0, 0xCF, 0x11, 0xE0]) || h.starts_with(b"{\\rtf")
}

/// Content first, then the file name.
pub fn detect(path: &Path) -> Option<Format> {
    sniff(path).or_else(|| Format::from_path(path))
}

/// What this PC's codecs can do; HEIC and AVIF depend on Microsoft Store extensions,
/// Word documents on Microsoft Word being installed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Caps {
    pub heic_decode: bool,
    pub heic_encode: bool,
    pub avif_decode: bool,
    pub word: bool,
}

impl Caps {
    /// Everything available; for tests of menu logic.
    pub const ALL: Caps = Caps { heic_decode: true, heic_encode: true, avif_decode: true, word: true };

    pub fn can_read(&self, f: Format) -> bool {
        match f {
            Format::Heic => self.heic_decode,
            Format::Avif => self.avif_decode,
            f if f.is_word() => self.word,
            _ => true,
        }
    }

    /// DOCX is always writable: through Word as editable text, otherwise as pictures.
    pub fn can_write(&self, f: Format) -> bool {
        match f {
            Format::Heic => self.heic_encode,
            Format::Gif | Format::Doc | Format::Rtf | Format::Odt => false,
            _ => true,
        }
    }
}
