//! How an uploaded image comes back.

use axum::body::Bytes;
use castle::app::App;
use loco_rs::testing::prelude::*;
use serial_test::serial;

use super::authz::{bearer, fresh_db, seed};

const PNG: &[u8] = b"\x89PNG\r\n\x1a\n not much of a picture";

#[tokio::test]
#[serial]
async fn an_upload_is_served_as_an_image_and_nothing_else() {
    fresh_db();
    request::<App, _, _>(|request, ctx| async move {
        let f = seed(&ctx).await;
        let (h, v) = bearer(&f.staff_token);
        let form = [
            b"--b\r\nContent-Disposition: form-data; name=\"file\"; filename=\"x.html\"\r\nContent-Type: text/html\r\n\r\n".as_slice(),
            PNG,
            b"\r\n--b--\r\n",
        ]
        .concat();
        let stored: serde_json::Value = request
            .post("/api/uploads")
            .add_header(h, &v)
            .content_type("multipart/form-data; boundary=b")
            .bytes(Bytes::from(form))
            .await
            .json();
        let url = stored["url"].as_str().expect("where it is");
        // The bytes decide what it is, not the name or type the sender gave.
        assert!(url.ends_with(".png"), "{url}");

        let response = request.get(url).add_header(h, &v).await;
        assert_eq!(response.status_code(), 200);
        assert_eq!(response.header("content-type"), "image/png");
        assert_eq!(response.header("x-content-type-options"), "nosniff");
        assert_eq!(response.header("content-disposition"), "inline");
        // Its own policy, not the pages': it loads nothing and runs nothing.
        assert_eq!(
            response.header("content-security-policy"),
            "default-src 'none'; sandbox"
        );
        assert_eq!(response.as_bytes().as_ref(), PNG);
    })
    .await;
}
