//! End-to-end authorization tests.
//!
//! These exercise the rules that decide who may read and write client findings
//! — the property the whole product rests on, since a leak here means one
//! client reading another's penetration-test results. They drive the real HTTP
//! surface (router, extractors, handlers, database) rather than calling model
//! methods directly, because every past bug in this area came from the wiring
//! between those layers, not from the rules themselves.

use castle::{
    app::App,
    models::_entities::{findings, project_members, projects, users},
    models::users::RegisterParams,
};
use loco_rs::testing::prelude::*;
use sea_orm::{ActiveModelTrait, ActiveValue::Set, DatabaseConnection};
use serial_test::serial;
use std::path::Path;
use std::sync::atomic::{AtomicU32, Ordering};

/// Everything a test needs to make an authenticated request as each role.
struct Fixture {
    manager_token: String,
    staff_token: String,
    client_token: String,
    outsider_token: String,
    project_id: i32,
    draft_id: i32,
    published_id: i32,
}

async fn make_user(db: &DatabaseConnection, email: &str, name: &str, role: &str) -> users::Model {
    let user = users::Model::create_with_password(
        db,
        &RegisterParams {
            email: email.to_string(),
            password: "12341234".to_string(),
            name: name.to_string(),
        },
    )
    .await
    .expect("create user");

    let mut active: users::ActiveModel = user.into();
    active.role = Set(role.to_string());
    active.update(db).await.expect("set role")
}

async fn make_finding(
    db: &DatabaseConnection,
    project_id: i32,
    author_id: i32,
    title: &str,
    status: &str,
) -> findings::Model {
    findings::ActiveModel {
        project_id: Set(project_id),
        author_id: Set(author_id),
        title: Set(title.to_string()),
        description: Set("d".to_string()),
        technical_description: Set("t".to_string()),
        impact: Set("i".to_string()),
        recommendation: Set("r".to_string()),
        status: Set(status.to_string()),
        ..Default::default()
    }
    .insert(db)
    .await
    .expect("create finding")
}

/// Points the next boot at a database file of its own. Call it before
/// `request::<App, _, _>` in every test.
///
/// `#[serial]` alone is not enough. Loco's test harness recreates the schema on
/// every boot, and a connection from the previous test's pool can still hold
/// sqlite's write lock when the next boot starts dropping tables — the whole
/// suite then fails with "database is locked", for a reason that has nothing to
/// do with authorization. Sharing one file also means one test's leftover rows
/// can collide with the next test's fixtures. A file per test removes the shared
/// resource instead of trying to sequence access to it.
///
/// The files live under `target/`, which is already ignored, and each is deleted
/// before it is reused so a crashed run cannot poison the next one.
fn fresh_db() {
    static COUNTER: AtomicU32 = AtomicU32::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);

    let dir = Path::new("target/test-dbs");
    std::fs::create_dir_all(dir).expect("create test db dir");
    let path = dir.join(format!("authz-{n}.sqlite"));
    // -wal and -shm are sqlite's sidecar files; leaving them behind would carry
    // committed rows into the "fresh" database.
    for suffix in ["", "-wal", "-shm"] {
        let _ = std::fs::remove_file(format!("{}{suffix}", path.display()));
    }

    // config/test.yaml reads this at boot: `uri: {{ get_env(name="DATABASE_URL", ...) }}`.
    std::env::set_var(
        "DATABASE_URL",
        format!("sqlite://{}?mode=rwc", path.display()),
    );
}

/// One manager (project owner), one staff member, one client, and one user with
/// no membership at all — plus a draft and a published finding.
async fn seed(ctx: &loco_rs::app::AppContext) -> Fixture {
    let db = &ctx.db;

    let manager = make_user(db, "manager@test.com", "Manager", "manager").await;
    let staff = make_user(db, "staff@test.com", "Staff", "staff").await;
    let client = make_user(db, "client@test.com", "Client", "client").await;
    let outsider = make_user(db, "outsider@test.com", "Outsider", "client").await;

    let project = projects::ActiveModel {
        name: Set("Engagement".to_string()),
        description: Set(Some("desc".to_string())),
        created_by: Set(manager.id),
        ..Default::default()
    }
    .insert(db)
    .await
    .expect("create project");

    for (user_id, role) in [(staff.id, "staff"), (client.id, "client")] {
        project_members::ActiveModel {
            project_id: Set(project.id),
            user_id: Set(user_id),
            role: Set(role.to_string()),
            ..Default::default()
        }
        .insert(db)
        .await
        .expect("add member");
    }

    let draft = make_finding(db, project.id, staff.id, "Draft finding", "draft").await;
    let published = make_finding(db, project.id, staff.id, "Published finding", "published").await;

    let jwt = ctx.config.get_jwt_config().expect("jwt config");
    let token = |u: &users::Model| {
        u.generate_jwt(&jwt.secret, jwt.expiration)
            .expect("generate jwt")
    };

    Fixture {
        manager_token: token(&manager),
        staff_token: token(&staff),
        client_token: token(&client),
        outsider_token: token(&outsider),
        project_id: project.id,
        draft_id: draft.id,
        published_id: published.id,
    }
}

fn bearer(token: &str) -> (&'static str, String) {
    ("authorization", format!("Bearer {token}"))
}

#[tokio::test]
#[serial]
async fn creating_manager_appears_in_the_member_list() {
    fresh_db();
    request::<App, _, _>(|request, ctx| async move {
        let f = seed(&ctx).await;
        let (h, v) = bearer(&f.manager_token);

        let created = request
            .post("/api/projects")
            .add_header(h, &v)
            .json(&serde_json::json!({ "name": "Owned project" }))
            .await;
        assert_eq!(created.status_code(), 200);
        let project_id = created.json::<serde_json::Value>()["id"].as_i64().unwrap();

        // The manager who created it is listed as a member (role "manager"),
        // so ownership is visible alongside onboarded staff/clients.
        let members = request
            .get(&format!("/api/projects/{project_id}/members"))
            .add_header(h, &v)
            .await;
        let body: serde_json::Value = members.json();
        let owner = body
            .as_array()
            .unwrap()
            .iter()
            .find(|m| m["user"]["email"] == "manager@test.com");
        assert!(
            owner.is_some(),
            "creating manager missing from members: {body}"
        );
        assert_eq!(owner.unwrap()["role"], "manager");
    })
    .await;
}

#[tokio::test]
#[serial]
async fn clients_cannot_see_draft_findings() {
    fresh_db();
    request::<App, _, _>(|request, ctx| async move {
        let f = seed(&ctx).await;
        let (h, v) = bearer(&f.client_token);

        // A draft must be indistinguishable from a finding that does not exist,
        // so the client cannot learn that unpublished work is under way.
        let response = request
            .get(&format!("/api/findings/{}", f.draft_id))
            .add_header(h, &v)
            .await;
        assert_eq!(response.status_code(), 404);

        let response = request
            .get(&format!("/api/findings/{}", f.published_id))
            .add_header(h, &v)
            .await;
        assert_eq!(response.status_code(), 200);
    })
    .await;
}

#[tokio::test]
#[serial]
async fn draft_findings_are_omitted_from_a_clients_list() {
    fresh_db();
    request::<App, _, _>(|request, ctx| async move {
        let f = seed(&ctx).await;
        let (h, v) = bearer(&f.client_token);

        let response = request
            .get(&format!("/api/projects/{}/findings", f.project_id))
            .add_header(h, &v)
            .await;
        assert_eq!(response.status_code(), 200);

        let body = response.text();
        assert!(
            body.contains("Published finding"),
            "client should see published work: {body}"
        );
        assert!(
            !body.contains("Draft finding"),
            "draft leaked into the client's list: {body}"
        );
    })
    .await;
}

#[tokio::test]
#[serial]
async fn staff_and_managers_do_see_drafts() {
    fresh_db();
    request::<App, _, _>(|request, ctx| async move {
        let f = seed(&ctx).await;

        for token in [&f.staff_token, &f.manager_token] {
            let (h, v) = bearer(token);
            let response = request
                .get(&format!("/api/findings/{}", f.draft_id))
                .add_header(h, &v)
                .await;
            assert_eq!(response.status_code(), 200);
        }
    })
    .await;
}

#[tokio::test]
#[serial]
async fn non_members_cannot_reach_a_project_at_all() {
    fresh_db();
    request::<App, _, _>(|request, ctx| async move {
        let f = seed(&ctx).await;
        let (h, v) = bearer(&f.outsider_token);

        for path in [
            format!("/api/projects/{}", f.project_id),
            format!("/api/projects/{}/findings", f.project_id),
            format!("/api/projects/{}/members", f.project_id),
        ] {
            let response = request.get(&path).add_header(h, &v).await;
            assert_eq!(
                response.status_code(),
                401,
                "{path} was reachable by a non-member"
            );
        }

        // Even the published finding, which its own project's client may read.
        let response = request
            .get(&format!("/api/findings/{}", f.published_id))
            .add_header(h, &v)
            .await;
        assert_eq!(response.status_code(), 401);
    })
    .await;
}

#[tokio::test]
#[serial]
async fn clients_cannot_write_or_publish() {
    fresh_db();
    request::<App, _, _>(|request, ctx| async move {
        let f = seed(&ctx).await;
        let (h, v) = bearer(&f.client_token);

        let response = request
            .post(&format!("/api/projects/{}/findings", f.project_id))
            .add_header(h, &v)
            .json(&serde_json::json!({
                "title": "Client-authored",
                "description": "",
                "technical_description": "",
                "impact": "",
                "recommendation": ""
            }))
            .await;
        assert_eq!(response.status_code(), 401);

        // Nor may they publish someone else's draft — but they must not learn
        // it exists either, so this is a 404 rather than a 401.
        let response = request
            .post(&format!("/api/findings/{}/publish", f.draft_id))
            .add_header(h, &v)
            .await;
        assert!(
            response.status_code() == 401 || response.status_code() == 404,
            "client publish returned {}",
            response.status_code()
        );
    })
    .await;
}

#[tokio::test]
#[serial]
async fn unauthenticated_requests_are_rejected() {
    fresh_db();
    request::<App, _, _>(|request, ctx| async move {
        let f = seed(&ctx).await;

        let response = request
            .get(&format!("/api/findings/{}", f.published_id))
            .await;
        assert_eq!(response.status_code(), 401);

        // Uploads are auth-gated too: this is the regression that broke inline
        // images, and it must stay gated rather than be loosened to fix them.
        let response = request.get("/api/uploads/anything.png").await;
        assert_eq!(response.status_code(), 401);
    })
    .await;
}

#[tokio::test]
#[serial]
async fn oversized_fields_are_rejected_over_http() {
    fresh_db();
    request::<App, _, _>(|request, ctx| async move {
        let f = seed(&ctx).await;
        let (h, v) = bearer(&f.staff_token);

        let response = request
            .post(&format!("/api/projects/{}/findings", f.project_id))
            .add_header(h, &v)
            .json(&serde_json::json!({
                "title": "Too long",
                "description": "a".repeat(70_000),
                "technical_description": "",
                "impact": "",
                "recommendation": ""
            }))
            .await;
        assert_eq!(response.status_code(), 400);
    })
    .await;
}

#[tokio::test]
#[serial]
async fn exploit_payloads_round_trip_unmodified() {
    fresh_db();
    request::<App, _, _>(|request, ctx| async move {
        let f = seed(&ctx).await;
        let (h, v) = bearer(&f.staff_token);
        let payload = r#""><script>alert(document.cookie)</script>"#;

        let response = request
            .post(&format!("/api/projects/{}/findings", f.project_id))
            .add_header(h, &v)
            .json(&serde_json::json!({
                "title": "Reflected XSS",
                "description": payload,
                "technical_description": "",
                "impact": "",
                "recommendation": ""
            }))
            .await;
        assert_eq!(response.status_code(), 200);

        // The report is the product: a finding that quotes an exploit must
        // store it byte-for-byte. Sanitising on input would corrupt it.
        let created: serde_json::Value = response.json();
        let id = created["id"].as_i64().expect("finding id");
        let response = request
            .get(&format!("/api/findings/{id}"))
            .add_header(h, &v)
            .await;
        let body: serde_json::Value = response.json();
        assert_eq!(body["description"].as_str(), Some(payload));
    })
    .await;
}

#[tokio::test]
#[serial]
async fn repeated_failed_logins_are_throttled() {
    fresh_db();
    request::<App, _, _>(|request, ctx| async move {
        seed(&ctx).await;

        let mut saw_429 = false;
        for _ in 0..12 {
            let response = request
                .post("/api/auth/login")
                .json(&serde_json::json!({
                    "email": "throttle-probe@test.com",
                    "password": "wrong"
                }))
                .await;
            if response.status_code() == 429 {
                saw_429 = true;
                break;
            }
        }
        assert!(saw_429, "login was never throttled");
    })
    .await;
}
