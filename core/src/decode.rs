//! Loading images: the `image` crate first (with EXIF rotation applied and the colour
//! profile kept), Windows codecs for HEIC/AVIF or anything it can't read.
//! The format is taken from the file's content, not its name, and damaged files are
//! refused with a plain-English reason rather than half-decoded.

use std::path::Path;

use anyhow::{bail, Result};
use image::{DynamicImage, ImageDecoder, ImageReader};

use crate::formats::{self, Format};
use crate::wic;

pub struct Decoded {
    pub image: DynamicImage,
    /// Embedded ICC colour profile, passed on to outputs so colours don't shift.
    pub icc: Option<Vec<u8>>,
}

/// Whether a JPEG's image data runs to its end marker. Decoders happily turn a
/// cut-off JPEG (failed download, full disk) into a picture with a grey block.
pub fn jpeg_is_complete(data: &[u8]) -> bool {
    let mut i = 2;
    // Walk the header segments (skipping embedded thumbnails) to the main image data.
    while i + 4 <= data.len() {
        if data[i] != 0xFF {
            return false;
        }
        let marker = data[i + 1];
        if marker == 0xFF {
            i += 1;
            continue;
        }
        if marker == 0xDA {
            return data[i..].windows(2).any(|w| w == [0xFF, 0xD9]);
        }
        let len = u16::from_be_bytes([data[i + 2], data[i + 3]]) as usize;
        i += 2 + len;
    }
    false
}

fn via_image_crate(path: &Path) -> Result<Decoded> {
    let mut reader = ImageReader::open(path)?.with_guessed_format()?;
    // Large photos are fine; the default limit (512 MB) would refuse ~100 MP images.
    reader.no_limits();
    let mut decoder = reader.into_decoder()?;
    let icc = decoder.icc_profile().ok().flatten();
    let orientation = decoder.orientation().ok();
    let mut image = DynamicImage::from_decoder(decoder)?;
    if let Some(o) = orientation {
        image.apply_orientation(o);
    }
    Ok(Decoded { image, icc })
}

pub fn load(path: &Path) -> Result<Decoded> {
    let len = std::fs::metadata(path)?.len();
    if len == 0 {
        bail!("the file is empty");
    }
    let real = formats::sniff(path);
    let named = Format::from_path(path);
    if real == Some(Format::Jpg) && !jpeg_is_complete(&std::fs::read(path)?) {
        bail!("this JPEG is incomplete (the file was cut off)");
    }
    let what = real.or(named).map(|f| f.label()).unwrap_or_else(|| "image".into());

    // HEIC/AVIF go straight to Windows' codecs; everything else tries the image crate first.
    let native_first = matches!(real.or(named), Some(Format::Heic | Format::Avif));
    let mut first_error = None;
    if !native_first {
        match via_image_crate(path) {
            Ok(d) => return Ok(d),
            Err(e) => first_error = Some(e),
        }
    }
    match wic::decode(path) {
        Ok(image) => Ok(Decoded { image, icc: None }),
        Err(e) => match (real, first_error) {
            // Nothing recognised the content at all.
            (None, _) => bail!("this isn't a picture File Beaver can read"),
            // A known format that our decoder rejected: it's damaged.
            (Some(_), Some(_)) => bail!("this {what} file is damaged and can't be opened"),
            // HEIC/AVIF without the codec: Windows' message says which one.
            (Some(_), None) => Err(e),
        },
    }
}
