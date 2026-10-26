//! What the ring offers for a given set of dragged files. Pure logic, no Windows calls.
//!
//! Page 0 is the Convert ring (target formats), page 1 the Tools ring. Order is clockwise
//! from the top, matching the reference app (for a JPG: PNG at the top, DOCX top-left).

use std::path::Path;

use serde::Serialize;

use crate::formats::{Caps, Format};

#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct Segment {
    /// "convert:png" or "tool:crop"; this is what a drop reports.
    pub id: String,
    pub label: String,
    /// Icon key the ring UI maps to an icon; None for format labels.
    pub icon: Option<&'static str>,
    pub enabled: bool,
}

#[derive(Serialize, Clone, Debug)]
pub struct Menu {
    pub pages: [Vec<Segment>; 2],
    pub start_page: usize,
    /// File name, or "3 files".
    pub title: String,
}

pub const CONVERT_PAGE: usize = 0;
pub const TOOLS_PAGE: usize = 1;

const TARGETS: [Format; 9] = [
    Format::Jpg,
    Format::Png,
    Format::Webp,
    Format::Heic,
    Format::Tiff,
    Format::Avif,
    Format::Bmp,
    Format::Pdf,
    Format::Docx,
];

/// (id, label, icon)
const TOOLS: [(&str, &str, &str); 7] = [
    ("compress", "Compress", "compress"),
    ("metadata", "Metadata", "tag"),
    ("edit", "Edit", "sliders"),
    ("annotate", "Annotate", "pen"),
    ("add_bg", "Add BG", "image"),
    ("crop", "Crop", "crop"),
    ("redact", "Redact", "eye-off"),
];

fn convert_page(formats: &[Option<Format>], caps: &Caps) -> Vec<Segment> {
    let Some(known) = formats.iter().copied().collect::<Option<Vec<Format>>>() else { return Vec::new() };
    if known.is_empty() || known.iter().any(|f| !f.is_image() && *f != Format::Pdf) {
        return Vec::new();
    }
    let readable = known.iter().all(|f| caps.can_read(*f));
    // Hide the source's own format only when every file shares it.
    let skip = known.iter().all(|f| *f == known[0]).then_some(known[0]);
    TARGETS
        .iter()
        .filter(|f| Some(**f) != skip)
        .map(|f| Segment {
            id: format!("convert:{}", f.id()),
            label: f.label(),
            icon: None,
            enabled: readable && caps.can_write(*f),
        })
        .collect()
}

fn tool_enabled(tool: &str, formats: &[Option<Format>], caps: &Caps) -> bool {
    let Some(known) = formats.iter().copied().collect::<Option<Vec<Format>>>() else { return false };
    if known.is_empty() || !known.iter().all(|f| caps.can_read(*f)) {
        return false;
    }
    let all = |ok: &dyn Fn(Format) -> bool| known.iter().all(|f| ok(*f));
    match tool {
        // Batch-safe quick actions.
        "compress" => all(&|f| matches!(f, Format::Jpg | Format::Png | Format::Webp | Format::Tiff | Format::Bmp | Format::Pdf)),
        "metadata" => all(&|f| matches!(f, Format::Jpg | Format::Png | Format::Webp | Format::Tiff | Format::Pdf)),
        // Editors open one file at a time.
        _ if known.len() > 1 => false,
        "annotate" | "crop" | "redact" => known[0].is_image() || known[0] == Format::Pdf,
        _ => known[0].is_image(),
    }
}

fn tools_page(formats: &[Option<Format>], caps: &Caps) -> Vec<Segment> {
    TOOLS
        .iter()
        .map(|(id, label, icon)| Segment {
            id: format!("tool:{id}"),
            label: label.to_string(),
            icon: Some(icon),
            enabled: tool_enabled(id, formats, caps),
        })
        .collect()
}

pub fn build(paths: &[String], caps: &Caps) -> Menu {
    let formats: Vec<Option<Format>> = paths.iter().map(|p| Format::from_path(Path::new(p))).collect();
    let convert = convert_page(&formats, caps);
    let start_page = if convert.iter().any(|s| s.enabled) { CONVERT_PAGE } else { TOOLS_PAGE };
    let title = match paths {
        [one] => Path::new(one).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(),
        many => format!("{} files", many.len()),
    };
    Menu { pages: [convert, tools_page(&formats, caps)], start_page, title }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ids(segs: &[Segment]) -> Vec<&str> {
        segs.iter().map(|s| s.id.as_str()).collect()
    }

    fn enabled(segs: &[Segment]) -> Vec<&str> {
        segs.iter().filter(|s| s.enabled).map(|s| s.id.as_str()).collect()
    }

    #[test]
    fn jpg_convert_ring_matches_reference_order() {
        let m = build(&["C:\\x\\Page 001.jpg".into()], &Caps::ALL);
        assert_eq!(
            ids(&m.pages[CONVERT_PAGE]),
            ["convert:png", "convert:webp", "convert:heic", "convert:tiff", "convert:avif", "convert:bmp", "convert:pdf", "convert:docx"]
        );
        assert_eq!(m.start_page, CONVERT_PAGE);
        assert_eq!(m.title, "Page 001.jpg");
    }

    #[test]
    fn tools_ring_order_and_jpg_enables_all() {
        let m = build(&["a.jpg".into()], &Caps::ALL);
        assert_eq!(
            ids(&m.pages[TOOLS_PAGE]),
            ["tool:compress", "tool:metadata", "tool:edit", "tool:annotate", "tool:add_bg", "tool:crop", "tool:redact"]
        );
        assert_eq!(enabled(&m.pages[TOOLS_PAGE]).len(), 7);
    }

    #[test]
    fn pdf_skips_pdf_target_and_image_only_tools() {
        let m = build(&["doc.PDF".into()], &Caps::ALL);
        assert!(!ids(&m.pages[CONVERT_PAGE]).contains(&"convert:pdf"));
        assert!(ids(&m.pages[CONVERT_PAGE]).contains(&"convert:jpg"));
        assert_eq!(
            enabled(&m.pages[TOOLS_PAGE]),
            ["tool:compress", "tool:metadata", "tool:annotate", "tool:crop", "tool:redact"]
        );
    }

    #[test]
    fn mixed_batch_shows_all_formats_and_only_batch_tools() {
        let m = build(&["a.jpg".into(), "b.png".into()], &Caps::ALL);
        assert_eq!(m.pages[CONVERT_PAGE].len(), 9);
        assert_eq!(enabled(&m.pages[TOOLS_PAGE]), ["tool:compress", "tool:metadata"]);
        assert_eq!(m.title, "2 files");
    }

    #[test]
    fn unsupported_file_starts_on_tools_with_everything_disabled() {
        let m = build(&["notes.txt".into()], &Caps::ALL);
        assert!(m.pages[CONVERT_PAGE].is_empty());
        assert_eq!(m.start_page, TOOLS_PAGE);
        assert!(enabled(&m.pages[TOOLS_PAGE]).is_empty());
    }

    #[test]
    fn missing_heic_codecs_grey_out_heic() {
        let no_heic = Caps { heic_decode: false, heic_encode: false, avif_decode: true };
        let m = build(&["a.png".into()], &no_heic);
        let heic = m.pages[CONVERT_PAGE].iter().find(|s| s.id == "convert:heic").unwrap();
        assert!(!heic.enabled);
        // A HEIC file itself can't be opened at all.
        let m = build(&["IMG_0001.HEIC".into()], &no_heic);
        assert!(enabled(&m.pages[CONVERT_PAGE]).is_empty());
        assert!(enabled(&m.pages[TOOLS_PAGE]).is_empty());
    }

    #[test]
    fn gif_and_avif_have_no_compress_or_metadata() {
        let m = build(&["a.gif".into()], &Caps::ALL);
        assert!(!enabled(&m.pages[TOOLS_PAGE]).contains(&"tool:compress"));
        assert!(!enabled(&m.pages[TOOLS_PAGE]).contains(&"tool:metadata"));
    }
}
