//! The deployment's own logo, shown in the portal's header instead of the
//! built-in brand.
//!
//! It is a file the operator places on the host, named by `settings.logo`; there
//! is no way to set it through the web. The type is decided by the bytes, as for
//! uploads. Unlike uploads, SVG is accepted here, because the file comes from the
//! operator and not from a user. It is still never treated as a document:
//!
//! - the portal embeds it with `<img>`, where an SVG's scripts do not run and it
//!   cannot load anything;
//! - it is served as an attachment, so opening the URL directly downloads it
//!   instead of rendering it in this origin;
//! - `nosniff` applies as everywhere, and the file carries a policy of its
//!   own (`FILE_POLICY`) under which it would run and load nothing.
use axum::http::header::{
    CONTENT_DISPOSITION, CONTENT_SECURITY_POLICY, CONTENT_TYPE, X_CONTENT_TYPE_OPTIONS,
};
use axum::response::IntoResponse;
use loco_rs::prelude::*;

use super::uploads::sniff_image;
use crate::security::Settings;

/// Where the portal finds the logo when one is configured.
pub const LOGO_URL: &str = "/api/brand/logo";

/// A logo is a few kilobytes. The limit keeps a mistaken file from being read
/// into memory and sent with every page.
const MAX_BYTES: u64 = 1024 * 1024;

/// Whether `data` is an SVG document: an optional byte-order mark, then any XML
/// declaration, comments and a DOCTYPE without an internal subset, then the
/// `svg` root element.
fn is_svg(data: &[u8]) -> bool {
    let Ok(text) = std::str::from_utf8(data) else {
        return false;
    };
    let mut rest = text.trim_start_matches('\u{feff}').trim_start();
    loop {
        let after = |open: &str, close: &str| {
            let body = rest.strip_prefix(open)?;
            let end = body.find(close)?;
            Some((&body[..end], body[end + close.len()..].trim_start()))
        };
        rest = match after("<?xml", "?>")
            .or_else(|| after("<!--", "-->"))
            .or_else(|| after("<!DOCTYPE", ">").filter(|(doctype, _)| !doctype.contains('[')))
        {
            Some((_, tail)) => tail,
            None => break,
        };
    }
    rest.strip_prefix("<svg")
        .is_some_and(|tail| tail.starts_with(|c: char| c.is_ascii_whitespace() || c == '>'))
}

fn content_type(data: &[u8]) -> Option<&'static str> {
    match sniff_image(data) {
        Some("png") => Some("image/png"),
        Some("jpg") => Some("image/jpeg"),
        Some("gif") => Some("image/gif"),
        Some("webp") => Some("image/webp"),
        _ => is_svg(data).then_some("image/svg+xml"),
    }
}

/// Reads the logo and says what it is.
///
/// # Errors
/// When the file cannot be read, is larger than a logo has any reason to be, or
/// is not an image this serves.
pub async fn load(path: &str) -> Result<(&'static str, Vec<u8>)> {
    let refuse = |why: String| Error::Message(format!("logo {path}: {why}"));
    let size = tokio::fs::metadata(path)
        .await
        .map_err(|e| refuse(e.to_string()))?
        .len();
    if size > MAX_BYTES {
        return Err(refuse(format!(
            "{size} bytes, more than the {MAX_BYTES} a logo may be"
        )));
    }
    let data = tokio::fs::read(path)
        .await
        .map_err(|e| refuse(e.to_string()))?;
    let kind = content_type(&data)
        .ok_or_else(|| refuse("not an svg, png, jpeg, gif or webp image".to_string()))?;
    Ok((kind, data))
}

/// Needs no identity, like the auth mode: an `<img>` cannot send a bearer token,
/// and in proxy mode nothing reaches castle without a session anyway.
#[debug_handler]
async fn logo(State(ctx): State<AppContext>) -> Result<Response> {
    let settings = Settings::from_ctx(&ctx);
    let Some(path) = settings.logo_path() else {
        return Err(Error::NotFound);
    };
    // Read on every request, so replacing the file is all it takes to change it.
    let (kind, data) = load(path).await.map_err(|e| {
        tracing::error!(error = %e, "the logo was accepted at boot and cannot be served now");
        Error::NotFound
    })?;
    Ok((
        [
            (CONTENT_TYPE, kind),
            (X_CONTENT_TYPE_OPTIONS, "nosniff"),
            (CONTENT_DISPOSITION, "attachment"),
            (CONTENT_SECURITY_POLICY, super::FILE_POLICY),
        ],
        data,
    )
        .into_response())
}

pub fn routes() -> Routes {
    Routes::new().prefix("/api/brand").add("/logo", get(logo))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn raster_images_are_recognised_by_their_bytes() {
        assert_eq!(
            content_type(&[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A, 0]),
            Some("image/png")
        );
        assert_eq!(content_type(&[0xFF, 0xD8, 0xFF, 0xE0]), Some("image/jpeg"));
        assert_eq!(content_type(b"GIF89a.."), Some("image/gif"));
        assert_eq!(content_type(b"RIFF\0\0\0\0WEBPVP8 "), Some("image/webp"));
    }

    #[test]
    fn svg_is_recognised_behind_its_prolog() {
        for svg in [
            "<svg xmlns=\"http://www.w3.org/2000/svg\"/>".to_string(),
            "<svg>\n</svg>".to_string(),
            "\u{feff}<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<!-- exported -->\n<!DOCTYPE svg PUBLIC \"-//W3C//DTD SVG 1.1//EN\" \"http://www.w3.org/Graphics/SVG/1.1/DTD/svg11.dtd\">\n<svg viewBox=\"0 0 1 1\"></svg>".to_string(),
        ] {
            assert_eq!(content_type(svg.as_bytes()), Some("image/svg+xml"), "{svg}");
        }
    }

    #[test]
    fn anything_else_is_refused() {
        for other in [
            "<html><body><svg></svg></body></html>",
            "<svgx></svgx>",
            "<!DOCTYPE svg [<!ENTITY a \"aaaa\">]><svg>&a;</svg>",
            "<?xml version=\"1.0\"?><html/>",
            "plain text that mentions <svg>",
            "",
        ] {
            assert_eq!(content_type(other.as_bytes()), None, "{other}");
        }
        assert_eq!(content_type(&[0xFF, 0xFE, b'<', 0, b's', 0]), None);
    }
}
