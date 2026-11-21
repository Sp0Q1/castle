//! The deployment's own logo: announced to the SPA, served so that it can only
//! ever be an image, and refused at boot when it is not one.

use std::path::{Path, PathBuf};

use castle::app::App;
use loco_rs::testing::prelude::*;
use serial_test::serial;

use super::authz::fresh_db;

/// A logo an operator would not use, but a file a browser must not run.
const SVG: &str = r#"<?xml version="1.0"?>
<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 40 10" onload="alert(1)"><script>alert(2)</script><rect width="40" height="10"/></svg>"#;
const PNG: &[u8] = &[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A, 0, 0, 0, 0];

/// Points the next boot at a logo file with this content. Dropped, later boots
/// have none, even if the test panicked.
struct Logo(PathBuf);

impl Logo {
    fn containing(content: &[u8]) -> Self {
        let dir = Path::new("target/test-logos");
        std::fs::create_dir_all(dir).expect("create logo dir");
        let path = dir.join("logo");
        std::fs::write(&path, content).expect("write logo");
        std::env::set_var("CASTLE_LOGO", &path);
        Self(path)
    }
}

impl Drop for Logo {
    fn drop(&mut self) {
        std::env::remove_var("CASTLE_LOGO");
        let _ = std::fs::remove_file(&self.0);
    }
}

#[tokio::test]
#[serial]
async fn without_a_logo_the_portal_keeps_its_own_brand() {
    fresh_db();
    request::<App, _, _>(|request, _ctx| async move {
        let about: serde_json::Value = request.get("/api/auth/mode").await.json();
        assert_eq!(about["logo"], serde_json::Value::Null);

        let response = request.get("/api/brand/logo").await;
        assert_eq!(response.status_code(), 404);
    })
    .await;
}

#[tokio::test]
#[serial]
async fn a_logo_is_announced_and_served_as_an_image_and_nothing_else() {
    fresh_db();
    let logo = Logo::containing(SVG.as_bytes());
    let path = logo.0.clone();
    request::<App, _, _>(|request, _ctx| async move {
        let about: serde_json::Value = request.get("/api/auth/mode").await.json();
        assert_eq!(about["logo"], "/api/brand/logo");

        // No session and no token: an <img> cannot send one.
        let response = request.get("/api/brand/logo").await;
        assert_eq!(response.status_code(), 200);
        assert_eq!(response.header("content-type"), "image/svg+xml");
        // Opened directly, the file is downloaded, never rendered in this origin.
        assert_eq!(response.header("content-disposition"), "attachment");
        assert_eq!(response.header("x-content-type-options"), "nosniff");
        // And if a browser rendered it all the same, it would run nothing.
        assert_eq!(
            response.header("content-security-policy"),
            "default-src 'none'; sandbox"
        );
        assert_eq!(response.text(), SVG);

        // Replacing the file is all it takes; the type follows the bytes.
        std::fs::write(&path, PNG).expect("replace logo");
        let response = request.get("/api/brand/logo").await;
        assert_eq!(response.header("content-type"), "image/png");
    })
    .await;
}
