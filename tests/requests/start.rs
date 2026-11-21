//! What stops the app from starting.
//!
//! These call `create_app`, as loco's `start` command does. `Hooks::boot` is the
//! test harness's way in, not what a deployment runs.

use std::path::Path;

use castle::app::App;
use loco_rs::{
    app::Hooks,
    boot::{create_app, StartMode},
    environment::Environment,
};
use migration::Migrator;
use serial_test::serial;

use super::authz::fresh_db;

/// Starts the way `castle-cli start` does, with these variables set meanwhile.
async fn start_with(vars: &[(&str, &str)]) -> loco_rs::Result<()> {
    for (name, value) in vars {
        std::env::set_var(name, value);
    }
    let config = App::load_config(&Environment::Test).await;
    let started = match config {
        Ok(config) => {
            create_app::<App, Migrator>(StartMode::ServerOnly, &Environment::Test, config)
                .await
                .map(|_| ())
        }
        Err(e) => Err(e),
    };
    for (name, _) in vars {
        std::env::remove_var(name);
    }
    started
}

#[tokio::test]
#[serial]
async fn a_malformed_auth_setting_stops_the_start() {
    fresh_db();
    assert!(start_with(&[]).await.is_ok(), "the defaults must start");

    // None of these is a mode, and there is no falling back to a default one.
    for mode in ["Proxy", "prox", ""] {
        let refused = start_with(&[("CASTLE_AUTH_MODE", mode)]).await;
        let why = refused.expect_err("a mode that does not exist must not start");
        assert!(
            why.to_string().contains("invalid `settings:`"),
            "{mode:?}: {why}"
        );
    }
}

#[tokio::test]
#[serial]
async fn a_logo_that_cannot_be_served_stops_the_start() {
    fresh_db();
    let dir = Path::new("target/test-logos");
    std::fs::create_dir_all(dir).expect("create logo dir");
    let path = dir.join("not-a-logo");
    let path = path.to_str().expect("utf-8 path");

    for not_a_logo in [
        b"<html><body><script>alert(1)</script></body></html>".to_vec(),
        b"MZ\x90\x00 an executable".to_vec(),
        [b"\x89PNG\r\n\x1a\n".to_vec(), vec![0; 1024 * 1024]].concat(),
    ] {
        std::fs::write(path, &not_a_logo).expect("write file");
        let refused = start_with(&[("CASTLE_LOGO", path)]).await;
        assert!(refused
            .expect_err("must not start")
            .to_string()
            .contains("logo"));
    }
    std::fs::remove_file(path).expect("remove file");
    assert!(
        start_with(&[("CASTLE_LOGO", path)]).await.is_err(),
        "a missing file"
    );
}
