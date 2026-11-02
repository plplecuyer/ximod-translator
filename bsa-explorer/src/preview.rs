//! Decode an extracted file's bytes into something previewable: readable text,
//! or an RGBA image (PNG/JPG/BMP/TGA via [`image`], and DDS textures — BC1..BC7
//! and uncompressed — via [`image_dds`]). Anything else is reported as
//! unsupported, with a hint to extract it and open it externally.

/// Largest side (px) a preview image is scaled down to, to bound memory/upload.
const MAX_PREVIEW_SIDE: u32 = 1024;
/// Largest text preview kept in memory.
const MAX_TEXT_BYTES: usize = 200_000;

/// A decoded RGBA image ready to upload as an egui texture.
pub struct PreviewImage {
    pub width: usize,
    pub height: usize,
    pub rgba: Vec<u8>,
    /// Short description (original dimensions / format), shown under the image.
    pub note: String,
}

/// The result of decoding a file for preview.
pub enum Preview {
    Text { body: String, truncated: bool },
    Image(PreviewImage),
    Unsupported { ext: String, size: usize },
    Error(String),
}

const TEXT_EXTS: &[&str] = &[
    "txt", "ini", "cfg", "json", "xml", "csv", "md", "log", "yaml", "yml", "toml", "psc", "html",
    "htm", "bat", "lua", "cmd", "fos", "config",
];
const IMAGE_EXTS: &[&str] = &["png", "jpg", "jpeg", "bmp", "tga"];

fn ext_of(path: &str) -> String {
    let name = path.rsplit('/').next().unwrap_or(path);
    match name.rsplit_once('.') {
        Some((_, e)) => e.to_ascii_lowercase(),
        None => String::new(),
    }
}

/// Decode `bytes` (already extracted/decompressed) for preview, dispatching on
/// the file's extension.
pub fn decode(path: &str, bytes: &[u8]) -> Preview {
    let ext = ext_of(path);
    if ext == "dds" {
        return decode_dds(bytes);
    }
    if IMAGE_EXTS.contains(&ext.as_str()) {
        return decode_image(bytes);
    }
    if TEXT_EXTS.contains(&ext.as_str()) {
        return decode_text(bytes);
    }
    Preview::Unsupported {
        ext,
        size: bytes.len(),
    }
}

fn decode_text(bytes: &[u8]) -> Preview {
    let truncated = bytes.len() > MAX_TEXT_BYTES;
    let slice = &bytes[..bytes.len().min(MAX_TEXT_BYTES)];
    // Strip a UTF-8 BOM if present, then decode lossily so a stray byte never
    // aborts the preview.
    let slice = slice.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(slice);
    let body = String::from_utf8_lossy(slice).into_owned();
    Preview::Text { body, truncated }
}

fn decode_image(bytes: &[u8]) -> Preview {
    match image::load_from_memory(bytes) {
        Ok(img) => Preview::Image(to_preview(img, String::new())),
        Err(e) => Preview::Error(format!("image illisible : {e}")),
    }
}

fn decode_dds(bytes: &[u8]) -> Preview {
    let dds = match ddsfile::Dds::read(bytes) {
        Ok(d) => d,
        Err(e) => return Preview::Error(format!("DDS illisible : {e}")),
    };
    match image_dds::image_from_dds(&dds, 0) {
        Ok(rgba) => {
            let (w, h) = (rgba.width(), rgba.height());
            let note = format!("{w}×{h}");
            Preview::Image(to_preview(image::DynamicImage::ImageRgba8(rgba), note))
        }
        Err(e) => Preview::Error(format!("décodage DDS impossible : {e}")),
    }
}

/// Convert a decoded image to RGBA, downscaling if larger than the preview cap.
fn to_preview(img: image::DynamicImage, mut note: String) -> PreviewImage {
    let (ow, oh) = (img.width(), img.height());
    if note.is_empty() {
        note = format!("{ow}×{oh}");
    }
    let scaled = if ow.max(oh) > MAX_PREVIEW_SIDE {
        note = format!("{note} (aperçu réduit)");
        img.thumbnail(MAX_PREVIEW_SIDE, MAX_PREVIEW_SIDE)
    } else {
        img
    };
    let rgba = scaled.to_rgba8();
    PreviewImage {
        width: rgba.width() as usize,
        height: rgba.height() as usize,
        rgba: rgba.into_raw(),
        note,
    }
}

#[cfg(test)]
mod tests {
    use super::{Preview, decode};

    #[test]
    fn text_files_decode_as_text() {
        match decode("readme.txt", b"Bonjour \xC3\xA9 monde") {
            Preview::Text { body, truncated } => {
                assert!(body.contains("Bonjour"));
                assert!(!truncated);
            }
            _ => panic!("expected text preview"),
        }
    }

    #[test]
    fn unknown_binary_is_unsupported() {
        match decode("mesh.nif", &[0u8, 1, 2, 3]) {
            Preview::Unsupported { ext, size } => {
                assert_eq!(ext, "nif");
                assert_eq!(size, 4);
            }
            _ => panic!("expected unsupported"),
        }
    }
}
