//! OPC packaging: content types, relationships and the zip container.
use std::io::Write;

pub struct Part {
    pub name: String,
    pub bytes: Vec<u8>,
}

pub fn zip(parts: &[Part]) -> zip::result::ZipResult<Vec<u8>> {
    let mut out = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    let options = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated);
    for part in parts {
        out.start_file(part.name.as_str(), options)?;
        out.write_all(&part.bytes)?;
    }
    Ok(out.finish()?.into_inner())
}

pub fn content_types(parts: &[Part]) -> String {
    let mut out = String::from(
        "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\
         <Types xmlns=\"http://schemas.openxmlformats.org/package/2006/content-types\">\
         <Default Extension=\"rels\" ContentType=\"application/vnd.openxmlformats-package.relationships+xml\"/>\
         <Default Extension=\"xml\" ContentType=\"application/xml\"/>\
         <Default Extension=\"png\" ContentType=\"image/png\"/>\
         <Default Extension=\"jpeg\" ContentType=\"image/jpeg\"/>\
         <Default Extension=\"jpg\" ContentType=\"image/jpeg\"/>\
         <Default Extension=\"gif\" ContentType=\"image/gif\"/>\
         <Default Extension=\"bmp\" ContentType=\"image/bmp\"/>\
         <Default Extension=\"tiff\" ContentType=\"image/tiff\"/>\
         <Default Extension=\"emf\" ContentType=\"image/x-emf\"/>\
         <Default Extension=\"wmf\" ContentType=\"image/x-wmf\"/>\
         <Default Extension=\"pict\" ContentType=\"image/pict\"/>",
    );
    let main = "application/vnd.openxmlformats-officedocument.wordprocessingml";
    for part in parts {
        let kind = match part.name.as_str() {
            "word/document.xml" => "document.main+xml",
            "word/styles.xml" => "styles+xml",
            "word/numbering.xml" => "numbering+xml",
            "word/settings.xml" => "settings+xml",
            "word/footnotes.xml" => "footnotes+xml",
            "word/endnotes.xml" => "endnotes+xml",
            "word/comments.xml" => "comments+xml",
            "word/fontTable.xml" => "fontTable+xml",
            n if n.starts_with("word/header") => "header+xml",
            n if n.starts_with("word/footer") => "footer+xml",
            _ => continue,
        };
        out.push_str(&format!(
            "<Override PartName=\"/{}\" ContentType=\"{main}.{kind}\"/>",
            part.name
        ));
    }
    out.push_str("</Types>");
    out
}

pub const ROOT_RELS: &str = "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\
<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\">\
<Relationship Id=\"rId1\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument\" Target=\"word/document.xml\"/>\
</Relationships>";

pub fn rels(entries: &[(String, &str, String)]) -> String {
    let mut out = String::from(
        "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\
         <Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\">",
    );
    for (id, kind, target) in entries {
        out.push_str(&format!(
            "<Relationship Id=\"{id}\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/{kind}\" Target=\"{target}\"/>"
        ));
    }
    out.push_str("</Relationships>");
    out
}

pub const NS: &str = "xmlns:w=\"http://schemas.openxmlformats.org/wordprocessingml/2006/main\" \
xmlns:r=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships\" \
xmlns:wp=\"http://schemas.openxmlformats.org/drawingml/2006/wordprocessingDrawing\" \
xmlns:a=\"http://schemas.openxmlformats.org/drawingml/2006/main\" \
xmlns:pic=\"http://schemas.openxmlformats.org/drawingml/2006/picture\" \
xmlns:wps=\"http://schemas.microsoft.com/office/word/2010/wordprocessingShape\" \
xmlns:wpg=\"http://schemas.microsoft.com/office/word/2010/wordprocessingGroup\" \
xmlns:mc=\"http://schemas.openxmlformats.org/markup-compatibility/2006\"";
