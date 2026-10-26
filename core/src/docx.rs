//! Images -> a Word document, one picture per page, scaled to fit an A4 page with 1" margins.
//! A .docx is a zip of a few XML parts plus the pictures.

use std::io::Write;

use anyhow::Result;
use zip::write::SimpleFileOptions;

const EMU_PER_INCH: f64 = 914_400.0;
const MAX_W_IN: f64 = 8.27 - 2.0; // A4 width minus margins
const MAX_H_IN: f64 = 11.69 - 2.0;

/// A picture to place: PNG or JPEG bytes and pixel size.
pub struct Picture {
    pub bytes: Vec<u8>,
    pub png: bool,
    pub width: u32,
    pub height: u32,
}

fn extent(p: &Picture) -> (u64, u64) {
    // Pictures are laid out at 96 dpi, shrunk to fit the page if needed.
    let (w_in, h_in) = (p.width as f64 / 96.0, p.height as f64 / 96.0);
    let scale = (MAX_W_IN / w_in).min(MAX_H_IN / h_in).min(1.0);
    ((w_in * scale * EMU_PER_INCH) as u64, (h_in * scale * EMU_PER_INCH) as u64)
}

pub fn write(pictures: &[Picture]) -> Result<Vec<u8>> {
    let mut zip = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    let opts = SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);

    zip.start_file("[Content_Types].xml", opts)?;
    zip.write_all(br#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Default Extension="png" ContentType="image/png"/><Default Extension="jpeg" ContentType="image/jpeg"/><Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/></Types>"#)?;

    zip.start_file("_rels/.rels", opts)?;
    zip.write_all(br#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/></Relationships>"#)?;

    let mut rels = String::from(r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">"#);
    let mut body = String::new();
    for (i, p) in pictures.iter().enumerate() {
        let n = i + 1;
        let file = format!("image{n}.{}", if p.png { "png" } else { "jpeg" });
        rels.push_str(&format!(
            r#"<Relationship Id="rIdImg{n}" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/image" Target="media/{file}"/>"#
        ));
        let (cx, cy) = extent(p);
        if i > 0 {
            body.push_str(r#"<w:p><w:r><w:br w:type="page"/></w:r></w:p>"#);
        }
        body.push_str(&format!(
            r#"<w:p><w:r><w:drawing><wp:inline distT="0" distB="0" distL="0" distR="0"><wp:extent cx="{cx}" cy="{cy}"/><wp:docPr id="{n}" name="Picture {n}"/><a:graphic><a:graphicData uri="http://schemas.openxmlformats.org/drawingml/2006/picture"><pic:pic><pic:nvPicPr><pic:cNvPr id="{n}" name="{file}"/><pic:cNvPicPr/></pic:nvPicPr><pic:blipFill><a:blip r:embed="rIdImg{n}"/><a:stretch><a:fillRect/></a:stretch></pic:blipFill><pic:spPr><a:xfrm><a:off x="0" y="0"/><a:ext cx="{cx}" cy="{cy}"/></a:xfrm><a:prstGeom prst="rect"><a:avLst/></a:prstGeom></pic:spPr></pic:pic></a:graphicData></a:graphic></wp:inline></w:drawing></w:r></w:p>"#
        ));
        zip.start_file(format!("word/media/{file}"), SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored))?;
        zip.write_all(&p.bytes)?;
    }
    rels.push_str("</Relationships>");

    zip.start_file("word/_rels/document.xml.rels", opts)?;
    zip.write_all(rels.as_bytes())?;

    zip.start_file("word/document.xml", opts)?;
    zip.write_all(format!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships" xmlns:wp="http://schemas.openxmlformats.org/drawingml/2006/wordprocessingDrawing" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" xmlns:pic="http://schemas.openxmlformats.org/drawingml/2006/picture"><w:body>{body}<w:sectPr><w:pgSz w:w="11906" w:h="16838"/><w:pgMar w:top="1440" w:right="1440" w:bottom="1440" w:left="1440" w:header="708" w:footer="708" w:gutter="0"/></w:sectPr></w:body></w:document>"#
    ).as_bytes())?;

    Ok(zip.finish()?.into_inner())
}
