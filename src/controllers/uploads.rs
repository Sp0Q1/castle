#![allow(clippy::missing_errors_doc)]
#![allow(clippy::unused_async)]
//! Inline image uploads for the rich-text fields.
//!
//! An authenticated user POSTs an image; it is stored under `uploads/` with an
//! unguessable name and served back so it can be embedded in markdown via
//! `![](/api/uploads/<name>)`.
//!
//! NOTE: the serve endpoint is intentionally NOT auth-gated, because `<img>`
//! tags cannot send the JWT. Access relies on the unguessable UUID name. For a
//! production security tool this should move to signed URLs or a cookie-scoped
//! check.
use std::path::Path as FsPath;

use axum::http::header::CONTENT_TYPE;
use axum::response::IntoResponse;
use loco_rs::prelude::*;

use crate::security::CurrentUser;
use serde::Serialize;
use uuid::Uuid;

const UPLOAD_DIR: &str = "uploads";
const MAX_BYTES: usize = 10 * 1024 * 1024; // 10 MiB

#[derive(Serialize)]
struct UploadResponse {
    url: String,
}

/// Resolves an allowed image extension from the filename or declared MIME type.
fn image_ext(filename: &str, content_type: Option<&str>) -> Option<&'static str> {
    let lower = filename.to_lowercase();
    let by_name = if lower.ends_with(".png") {
        Some("png")
    } else if lower.ends_with(".jpg") || lower.ends_with(".jpeg") {
        Some("jpg")
    } else if lower.ends_with(".gif") {
        Some("gif")
    } else if lower.ends_with(".webp") {
        Some("webp")
    } else if lower.ends_with(".svg") {
        Some("svg")
    } else {
        None
    };
    by_name.or_else(|| match content_type {
        Some("image/png") => Some("png"),
        Some("image/jpeg") => Some("jpg"),
        Some("image/gif") => Some("gif"),
        Some("image/webp") => Some("webp"),
        Some("image/svg+xml") => Some("svg"),
        _ => None,
    })
}

fn mime_for(name: &str) -> &'static str {
    let lower = name.to_lowercase();
    if lower.ends_with(".png") {
        "image/png"
    } else if lower.ends_with(".jpg") || lower.ends_with(".jpeg") {
        "image/jpeg"
    } else if lower.ends_with(".gif") {
        "image/gif"
    } else if lower.ends_with(".webp") {
        "image/webp"
    } else if lower.ends_with(".svg") {
        "image/svg+xml"
    } else {
        "application/octet-stream"
    }
}

fn is_safe_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() < 128
        && !name.contains("..")
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-' || c == '_')
}

#[debug_handler]
pub async fn upload(
    _user: CurrentUser,
    State(_ctx): State<AppContext>,
    mut multipart: Multipart,
) -> Result<Response> {
    while let Some(field) = multipart
        .next_field()
        .await
        .map_err(|e| Error::BadRequest(e.to_string()))?
    {
        let filename = field.file_name().unwrap_or_default().to_string();
        let content_type = field.content_type().map(str::to_string);
        let Some(ext) = image_ext(&filename, content_type.as_deref()) else {
            continue;
        };
        let data = field
            .bytes()
            .await
            .map_err(|e| Error::BadRequest(e.to_string()))?;
        if data.len() > MAX_BYTES {
            return Err(Error::BadRequest("image too large (max 10 MiB)".to_string()));
        }
        std::fs::create_dir_all(UPLOAD_DIR).map_err(|e| Error::string(&e.to_string()))?;
        let name = format!("{}.{ext}", Uuid::new_v4());
        std::fs::write(FsPath::new(UPLOAD_DIR).join(&name), &data)
            .map_err(|e| Error::string(&e.to_string()))?;
        return format::json(UploadResponse {
            url: format!("/api/uploads/{name}"),
        });
    }
    Err(Error::BadRequest("no image file in upload".to_string()))
}

#[debug_handler]
pub async fn serve(Path(name): Path<String>) -> Result<Response> {
    if !is_safe_name(&name) {
        return Err(Error::NotFound);
    }
    let Ok(bytes) = std::fs::read(FsPath::new(UPLOAD_DIR).join(&name)) else {
        return Err(Error::NotFound);
    };
    Ok(([(CONTENT_TYPE, mime_for(&name))], bytes).into_response())
}

pub fn routes() -> Routes {
    Routes::new()
        .prefix("/api/uploads")
        .add("/", post(upload))
        .add("/{name}", get(serve))
}
