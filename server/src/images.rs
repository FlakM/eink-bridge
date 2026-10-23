//! Images a document displays, as opposed to the ink the reader draws on it.
//!
//! A brief may carry pictures — a screenshot of a broken page, a sketch on paper, a photo
//! of a whiteboard — under `image/<name>`. These are the rules for the names and bytes the
//! server will store and serve, kept in one place so the CLI can refuse the same things
//! the server would refuse, with a better message and before the upload.

/// The longest image name the server will store. Long enough for a descriptive file name,
/// short enough that it cannot smuggle a path or a very long URL.
pub const IMAGE_NAME_MAX: usize = 96;

/// The content type for a document image, or `None` when the name may not be stored.
///
/// A name is one path segment of ASCII letters, digits, dot, dash or underscore with an
/// extension from a small allowlist. SVG is deliberately absent: it is a document that can
/// carry script, and these images are served from the page's own origin.
pub fn document_image_type(name: &str) -> Option<&'static str> {
    if name.len() > IMAGE_NAME_MAX || name.starts_with('.') {
        return None;
    }
    if !name
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_'))
    {
        return None;
    }
    match name.rsplit_once('.')?.1.to_ascii_lowercase().as_str() {
        "png" => Some("image/png"),
        "jpg" | "jpeg" => Some("image/jpeg"),
        "webp" => Some("image/webp"),
        "gif" => Some("image/gif"),
        _ => None,
    }
}

/// Whether the bytes are actually the format the name promises. A `.png` that is not a
/// PNG is refused rather than stored and served, so an upload cannot put a document of
/// another kind behind an image URL.
pub fn bytes_are_image(content_type: &str, bytes: &[u8]) -> bool {
    match content_type {
        "image/png" => bytes.starts_with(b"\x89PNG\r\n\x1a\n"),
        "image/jpeg" => bytes.starts_with(&[0xFF, 0xD8, 0xFF]),
        "image/gif" => bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a"),
        "image/webp" => bytes.len() >= 12 && &bytes[..4] == b"RIFF" && &bytes[8..12] == b"WEBP",
        _ => false,
    }
}

/// The human name for a stored image type, for messages that read like sentences.
pub fn format_label(content_type: &str) -> &'static str {
    match content_type {
        "image/png" => "PNG",
        "image/jpeg" => "JPEG",
        "image/webp" => "WebP",
        "image/gif" => "GIF",
        _ => "image",
    }
}

/// The part of a path a document refers to an image by: its last segment.
pub fn image_name(path: &str) -> &str {
    path.rsplit(['/', '\\']).next().unwrap_or(path)
}

/// The alt text a picture gets when the caller did not write one: the file name without
/// its extension, because that is what the reader would have typed.
pub fn alt_from_name(name: &str) -> &str {
    name.rsplit_once('.').map_or(name, |(stem, _)| stem)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_image_names_that_cannot_escape_their_directory_are_accepted() {
        for name in [
            "shot.png",
            "a-b_c.webp",
            "diagram.v2.JPEG",
            "x.gif",
            "ü.png",
        ] {
            let accepted = document_image_type(name).is_some();
            // Non-ASCII is refused rather than transliterated: the markdown reference and
            // the stored name must be the same bytes.
            assert_eq!(accepted, name.is_ascii(), "{name}");
        }
        for name in [
            "../secret.png",
            "sub/dir.png",
            ".hidden.png",
            "no-extension",
            "script.svg",
            "page.html",
            &"a".repeat(IMAGE_NAME_MAX) as &str,
        ] {
            assert!(document_image_type(name).is_none(), "{name} was accepted");
        }
    }

    #[test]
    fn bytes_must_match_the_extension_they_are_stored_under() {
        assert!(bytes_are_image("image/png", b"\x89PNG\r\n\x1a\n"));
        assert!(bytes_are_image("image/jpeg", &[0xFF, 0xD8, 0xFF, 0xE0]));
        assert!(bytes_are_image("image/gif", b"GIF89a...."));
        assert!(bytes_are_image("image/webp", b"RIFF0000WEBP"));
        assert!(!bytes_are_image("image/png", b"<svg xmlns=\"...\">"));
        assert!(!bytes_are_image("image/png", b""));
        assert!(!bytes_are_image("image/jpeg", b"\x89PNG\r\n\x1a\n"));
    }

    #[test]
    fn a_name_is_the_last_segment_of_a_path() {
        assert_eq!(image_name("/tmp/shot.png"), "shot.png");
        assert_eq!(image_name("C:\\Users\\me\\shot.png"), "shot.png");
        assert_eq!(image_name("shot.png"), "shot.png");
        assert_eq!(alt_from_name("diagram.v2.png"), "diagram.v2");
    }
}
