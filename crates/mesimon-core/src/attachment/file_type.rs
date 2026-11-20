use serde::{Deserialize, Serialize};

/// A file's on-disk suffix and the MIME type reported by read_attachment
/// both come from this closed set. A missing type denotes a legacy .bin.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum FileType {
    #[serde(rename = "application/pdf")]
    Pdf,
    #[serde(rename = "image/png")]
    Png,
    #[serde(rename = "image/jpeg")]
    Jpeg,
    #[serde(rename = "image/gif")]
    Gif,
    #[serde(rename = "image/webp")]
    Webp,
    #[serde(rename = "application/zip")]
    Zip,
    #[serde(rename = "application/vnd.openxmlformats-officedocument.wordprocessingml.document")]
    Docx,
    #[serde(rename = "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet")]
    Xlsx,
    #[serde(rename = "application/gzip")]
    Gzip,
    #[serde(rename = "text/plain")]
    Text,
    #[default]
    #[serde(rename = "application/octet-stream")]
    Unknown,
}

impl FileType {
    pub fn extension(self) -> &'static str {
        match self {
            Self::Pdf => "pdf",
            Self::Png => "png",
            Self::Jpeg => "jpg",
            Self::Gif => "gif",
            Self::Webp => "webp",
            Self::Zip => "zip",
            Self::Docx => "docx",
            Self::Xlsx => "xlsx",
            Self::Gzip => "gz",
            Self::Text => "txt",
            Self::Unknown => "bin",
        }
    }

    pub fn detect(bytes: &[u8]) -> Self {
        const SIGNATURES: &[(&[u8], FileType)] = &[
            (b"%PDF-", FileType::Pdf),
            (b"\x89PNG\r\n\x1a\n", FileType::Png),
            (b"\xff\xd8\xff", FileType::Jpeg),
            (b"GIF87a", FileType::Gif),
            (b"GIF89a", FileType::Gif),
            (b"\x1f\x8b\x08", FileType::Gzip),
        ];
        for (signature, kind) in SIGNATURES {
            if bytes.starts_with(signature) {
                return *kind;
            }
        }
        if bytes.starts_with(b"RIFF") && bytes.get(8..12) == Some(b"WEBP") {
            return Self::Webp;
        }
        if bytes.starts_with(b"PK\x03\x04") {
            return office_type(bytes).unwrap_or(Self::Zip);
        }
        if bytes.starts_with(b"PK\x05\x06") || bytes.starts_with(b"PK\x07\x08") {
            return Self::Zip;
        }
        if !bytes.is_empty()
            && std::str::from_utf8(bytes).is_ok_and(|s| {
                s.chars().all(|c| !c.is_control() || matches!(c, '\n' | '\r' | '\t' | '\x0c'))
            })
        {
            return Self::Text;
        }
        Self::Unknown
    }
}

/// Office packages share ZIP's signature. Read member names from its central
/// directory, without inflating data or mistaking a string in a payload for a name.
fn office_type(bytes: &[u8]) -> Option<FileType> {
    // The 22-byte end record can be followed by a comment of up to 65535 bytes.
    let tail = &bytes[bytes.len().saturating_sub(22 + u16::MAX as usize)..];
    let end = tail.windows(22).enumerate().rev().find_map(|(i, end)| {
        (end.starts_with(b"PK\x05\x06")
            && tail.len() - i - 22 == u16::from_le_bytes([end[20], end[21]]) as usize)
            .then_some(end)
    })?;
    let start = u32::from_le_bytes(end[16..20].try_into().ok()?) as usize;
    let size = u32::from_le_bytes(end[12..16].try_into().ok()?) as usize;
    let mut directory = bytes.get(start..)?.get(..size)?;
    let mut manifest = false;
    let mut kind = None;
    while !directory.is_empty() {
        let (header, body) = directory.split_first_chunk::<46>()?;
        if !header.starts_with(b"PK\x01\x02") {
            return None;
        }
        let name_len = u16::from_le_bytes([header[28], header[29]]) as usize;
        let extra_len = u16::from_le_bytes([header[30], header[31]]) as usize;
        let comment_len = u16::from_le_bytes([header[32], header[33]]) as usize;
        match body.get(..name_len)? {
            b"[Content_Types].xml" => manifest = true,
            b"word/document.xml" => kind = Some(FileType::Docx),
            b"xl/workbook.xml" => kind = Some(FileType::Xlsx),
            _ => {}
        }
        directory = body.get(name_len + extra_len + comment_len..)?;
    }
    if manifest {
        kind
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn signatures_precede_the_utf8_fallback() {
        for (bytes, kind) in [
            (b"GIF87a".as_slice(), FileType::Gif),
            (b"GIF89a", FileType::Gif),
            (b"\xff\xd8\xff\xe0", FileType::Jpeg),
            (b"RIFF\x10\0\0\0WEBP", FileType::Webp),
            (b"RIFF\x10\0\0\0WAVE", FileType::Unknown),
            (b"\x1f\x8b\x08", FileType::Gzip),
            (b"PK\x05\x06", FileType::Zip),
            (b"PK\x03\x04truncated", FileType::Zip),
            ("résumé\r\n".as_bytes(), FileType::Text),
            (b"a\0b", FileType::Unknown),
            (b"", FileType::Unknown),
        ] {
            assert_eq!(FileType::detect(bytes), kind);
        }
    }

    #[test]
    fn office_packages_are_recognized_by_member_names() {
        // Exercise just the directory the detector reads. Payload bytes that
        // look like a document part must not classify an ordinary ZIP as Office.
        for (name, kind) in [
            ("word/document.xml", FileType::Docx),
            ("xl/workbook.xml", FileType::Xlsx),
            ("notes.txt", FileType::Zip),
        ] {
            let mut bytes = b"PK\x03\x04word/document.xml".to_vec();
            let start = bytes.len() as u32;
            for member in ["[Content_Types].xml", name] {
                let mut header = [0; 46];
                header[..4].copy_from_slice(b"PK\x01\x02");
                header[28..30].copy_from_slice(&(member.len() as u16).to_le_bytes());
                bytes.extend(header);
                bytes.extend(member.as_bytes());
            }
            let mut end = [0; 22];
            end[..4].copy_from_slice(b"PK\x05\x06");
            end[12..16].copy_from_slice(&(bytes.len() as u32 - start).to_le_bytes());
            end[16..20].copy_from_slice(&start.to_le_bytes());
            bytes.extend(end);
            assert_eq!(FileType::detect(&bytes), kind);
        }
    }
}
