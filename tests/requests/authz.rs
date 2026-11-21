//! End-to-end authorization tests.
//!
//! These exercise the rules that decide who may read and write client findings
//! — the property the whole product rests on, since a leak here means one
//! client reading another's penetration-test results. They drive the real HTTP
//! surface (router, extractors, handlers, database) rather than calling model
//! methods directly, because the wiring between those layers is where such a
//! rule is most easily lost.

use castle::{
    app::App,
    models::_entities::{findings, project_members, projects, users},
    models::findings::{FindingStatus, Severity},
    models::project_members::MemberRole,
    models::users::{RegisterParams, UserStatus},
};
use loco_rs::{boot::run_task, task, testing::prelude::*};
use sea_orm::{
    ActiveModelTrait, ActiveValue::Set, ConnectionTrait, DatabaseConnection, EntityTrait,
};
use serial_test::serial;
use std::path::Path;
use std::sync::atomic::{AtomicU32, Ordering};

/// Everything a test needs to make an authenticated request as each role.
pub(super) struct Fixture {
    pub(super) manager_token: String,
    pub(super) staff_token: String,
    pub(super) client_token: String,
    pub(super) outsider_token: String,
    pub(super) project_id: i64,
    pub(super) draft_id: i64,
    pub(super) published_id: i64,
}

pub(super) async fn make_user(
    db: &DatabaseConnection,
    email: &str,
    name: &str,
    role: &str,
) -> users::Model {
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

pub(super) async fn make_finding(
    db: &DatabaseConnection,
    project_id: i64,
    author_id: i64,
    title: &str,
    status: FindingStatus,
) -> findings::Model {
    findings::ActiveModel {
        project_id: Set(project_id),
        author_id: Set(author_id),
        title: Set(title.to_string()),
        description: Set("d".to_string()),
        technical_description: Set("t".to_string()),
        impact: Set("i".to_string()),
        recommendation: Set("r".to_string()),
        status: Set(status),
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
pub(super) fn fresh_db() {
    // Only sqlite needs this. CI points DATABASE_URL at a real Postgres so that
    // loose sqlite typing cannot hide an INT4/INT8 mismatch again (PR #65);
    // overwriting it here unconditionally sent those runs back to sqlite and
    // quietly retired the guarantee. Postgres has no shared write-lock to dodge,
    // and `dangerously_recreate` rebuilds the schema on each serial boot.
    if std::env::var("DATABASE_URL").is_ok_and(|url| !url.starts_with("sqlite:")) {
        return;
    }

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
pub(super) async fn seed(ctx: &loco_rs::app::AppContext) -> Fixture {
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

    for (user_id, role) in [
        (staff.id, MemberRole::Staff),
        (client.id, MemberRole::Client),
    ] {
        project_members::ActiveModel {
            project_id: Set(project.id),
            user_id: Set(user_id),
            role: Set(role),
            ..Default::default()
        }
        .insert(db)
        .await
        .expect("add member");
    }

    let draft = make_finding(
        db,
        project.id,
        staff.id,
        "Draft finding",
        FindingStatus::Draft,
    )
    .await;
    let published = make_finding(
        db,
        project.id,
        staff.id,
        "Published finding",
        FindingStatus::Published,
    )
    .await;

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

pub(super) fn bearer(token: &str) -> (&'static str, String) {
    ("authorization", format!("Bearer {token}"))
}

/// Management is not on a project. A manager's access is the role's and covers
/// every project, so no membership stands for it and none can be taken away
/// while the access stays. Each project says who its managers are.
#[tokio::test]
#[serial]
async fn managers_are_listed_and_are_not_members() {
    fresh_db();
    request::<App, _, _>(|request, ctx| async move {
        let f = seed(&ctx).await;
        let (mh, mv) = bearer(&f.manager_token);
        let (sh, sv) = bearer(&f.staff_token);
        let (oh, ov) = bearer(&f.outsider_token);

        let created: serde_json::Value = request
            .post("/api/projects")
            .add_header(mh, &mv)
            .json(&serde_json::json!({ "name": "Owned project" }))
            .await
            .json();
        let owned = format!("/api/projects/{}", created["id"]);

        // Opening a project puts nobody on it, and the project names its managers.
        let members: Vec<serde_json::Value> = request
            .get(&format!("{owned}/members"))
            .add_header(mh, &mv)
            .await
            .json();
        assert!(
            members.is_empty(),
            "opening a project made a member: {members:?}"
        );
        let managers: Vec<serde_json::Value> = request
            .get(&format!("{owned}/managers"))
            .add_header(mh, &mv)
            .await
            .json();
        let emails: Vec<&str> = managers
            .iter()
            .filter_map(|m| m["email"].as_str())
            .collect();
        assert_eq!(emails, ["manager@test.com"]);

        // A manager cannot be put on a project.
        let onboarded = request
            .post(&format!("{owned}/members"))
            .add_header(mh, &mv)
            .json(&serde_json::json!({ "user_email": "manager@test.com", "role": "staff" }))
            .await;
        assert_eq!(onboarded.status_code(), 400);

        // Whoever is on a project sees its managers; whoever is not learns nothing.
        let shared = format!("/api/projects/{}", f.project_id);
        let seen = request
            .get(&format!("{shared}/managers"))
            .add_header(sh, &sv)
            .await;
        assert_eq!(seen.status_code(), 200);
        assert!(seen.text().contains("manager@test.com"));
        let hidden = request
            .get(&format!("{shared}/managers"))
            .add_header(oh, &ov)
            .await;
        assert_eq!(hidden.status_code(), 404);
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
            // 404 (not 401): a non-member must not be able to distinguish a
            // forbidden project from a nonexistent one.
            assert_eq!(
                response.status_code(),
                404,
                "{path} disclosed its existence to a non-member"
            );
        }

        // Even the published finding, which its own project's client may read —
        // to an outsider it simply does not exist.
        let response = request
            .get(&format!("/api/findings/{}", f.published_id))
            .add_header(h, &v)
            .await;
        assert_eq!(response.status_code(), 404);
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
        // 403: a client *is* a member of the project (hiding it is pointless),
        // but only staff/management may write findings.
        assert_eq!(response.status_code(), 403);

        // Nor may they publish someone else's draft — and since a client can't
        // see drafts, this is a 404 that never reveals the draft exists.
        let response = request
            .post(&format!("/api/findings/{}/publish", f.draft_id))
            .add_header(h, &v)
            .await;
        assert_eq!(response.status_code(), 404);
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

        // Uploads are auth-gated too. The portal loads inline images through
        // its API client for that reason; the gate is not to be loosened.
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

/// A severity outside the scale is refused outright rather than stored, or
/// quietly replaced by the column default.
#[tokio::test]
#[serial]
async fn an_unknown_severity_is_rejected() {
    fresh_db();
    request::<App, _, _>(|request, ctx| async move {
        let f = seed(&ctx).await;
        let (h, v) = bearer(&f.staff_token);

        let response = request
            .post(&format!("/api/projects/{}/findings", f.project_id))
            .add_header(h, &v)
            .json(&serde_json::json!({
                "title": "Bad severity",
                "description": "d",
                "technical_description": "t",
                "impact": "i",
                "recommendation": "r",
                "severity": "critical"
            }))
            .await;
        assert_eq!(response.status_code(), 400);
        assert!(
            response.text().contains("severity must be one of"),
            "the rejection should name the accepted values: {}",
            response.text()
        );

        let listed = request
            .get(&format!("/api/projects/{}/findings", f.project_id))
            .add_header(h, &v)
            .await;
        let titles: Vec<String> = listed
            .json::<serde_json::Value>()
            .as_array()
            .unwrap()
            .iter()
            .map(|f| f["title"].as_str().unwrap_or_default().to_string())
            .collect();
        assert!(
            !titles.iter().any(|t| t == "Bad severity"),
            "a finding with an invalid severity was stored anyway: {titles:?}"
        );
    })
    .await;
}

/// Onboarding grants staff or client capacity only. "manager" is not a value the
/// endpoint can accept, so a manager cannot be minted through it.
#[tokio::test]
#[serial]
async fn onboarding_cannot_mint_a_manager() {
    fresh_db();
    request::<App, _, _>(|request, ctx| async move {
        let f = seed(&ctx).await;
        let (h, v) = bearer(&f.manager_token);

        let response = request
            .post(&format!("/api/projects/{}/members", f.project_id))
            .add_header(h, &v)
            .json(&serde_json::json!({
                "user_email": "newcomer@test.com",
                "role": "manager"
            }))
            .await;
        assert_eq!(response.status_code(), 400);
        assert!(
            response.text().contains("role must be one of"),
            "the rejection should name the accepted roles: {}",
            response.text()
        );

        let members = request
            .get(&format!("/api/projects/{}/members", f.project_id))
            .add_header(h, &v)
            .await;
        let body: serde_json::Value = members.json();
        assert!(
            !body
                .as_array()
                .unwrap()
                .iter()
                .any(|m| m["user"]["email"] == "newcomer@test.com"),
            "onboarding with role=manager created a membership: {body}"
        );
    })
    .await;
}

/// The typed columns must still serialize as the lowercase strings the SPA
/// reads — changing the enum representation would silently break every client.
#[tokio::test]
#[serial]
async fn severity_and_status_stay_lowercase_strings_on_the_wire() {
    fresh_db();
    request::<App, _, _>(|request, ctx| async move {
        let f = seed(&ctx).await;
        let (h, v) = bearer(&f.staff_token);

        let created = request
            .post(&format!("/api/projects/{}/findings", f.project_id))
            .add_header(h, &v)
            .json(&serde_json::json!({
                "title": "Wire format",
                "description": "d",
                "technical_description": "t",
                "impact": "i",
                "recommendation": "r",
                "severity": "elevated"
            }))
            .await;
        assert_eq!(created.status_code(), 200);
        let body: serde_json::Value = created.json();
        assert_eq!(body["severity"], "elevated");
        assert_eq!(body["status"], "draft");

        let published = request
            .post(&format!(
                "/api/findings/{}/publish",
                body["id"].as_i64().unwrap()
            ))
            .add_header(h, &v)
            .await;
        assert_eq!(published.json::<serde_json::Value>()["status"], "published");

        let members = request
            .get(&format!("/api/projects/{}/members", f.project_id))
            .add_header(h, &v)
            .await;
        let roles: Vec<String> = members
            .json::<serde_json::Value>()
            .as_array()
            .unwrap()
            .iter()
            .map(|m| m["role"].as_str().unwrap_or_default().to_string())
            .collect();
        assert!(
            roles
                .iter()
                .all(|r| ["staff", "client"].contains(&r.as_str())),
            "member roles changed shape on the wire: {roles:?}"
        );
    })
    .await;
}

/// Rows written before these columns were typed must still load. The enums are
/// stored as the same lowercase strings they always were, so a live database
/// needs no migration — this fails the moment that stops being true.
#[tokio::test]
#[serial]
async fn rows_written_as_plain_strings_still_load() {
    fresh_db();
    request::<App, _, _>(|_request, ctx| async move {
        let f = seed(&ctx).await;
        let db = &ctx.db;

        for sql in [
            format!(
                "UPDATE findings SET severity = 'extreme', status = 'published' WHERE id = {}",
                f.draft_id
            ),
            format!(
                "UPDATE project_members SET role = 'client' WHERE project_id = {}",
                f.project_id
            ),
            "UPDATE users SET status = 'invited' WHERE email = 'client@test.com'".to_string(),
        ] {
            db.execute_unprepared(&sql)
                .await
                .expect("write legacy value");
        }

        let finding = findings::Entity::find_by_id(f.draft_id)
            .one(db)
            .await
            .expect("load finding")
            .expect("finding exists");
        assert_eq!(finding.severity, Severity::Extreme);
        assert_eq!(finding.status, FindingStatus::Published);

        let members = project_members::Entity::find()
            .all(db)
            .await
            .expect("load members");
        assert!(
            !members.is_empty() && members.iter().all(|m| m.role == MemberRole::Client),
            "membership roles did not load from their stored strings"
        );

        let invited = users::Entity::find()
            .all(db)
            .await
            .expect("load users")
            .into_iter()
            .filter(|u| u.status == castle::models::users::UserStatus::Invited)
            .count();
        assert_eq!(
            invited, 1,
            "user status did not load from its stored string"
        );
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

/// The email-verification gate that closes the account pre-registration hijack.
/// Registration is open and unauthenticated, so anyone can create an account for
/// any address — but until that address is verified, the account must never get
/// a session, even with the correct password. Otherwise an attacker pre-registers
/// a victim's email and, once a manager onboards that address, logs in and reads
/// the project's confidential findings.
#[tokio::test]
#[serial]
async fn unverified_accounts_cannot_log_in() {
    fresh_db();
    request::<App, _, _>(|request, ctx| async move {
        // Registered but never verified — exactly the state /api/auth/register
        // leaves behind (create_with_password does not set email_verified_at).
        users::Model::create_with_password(
            &ctx.db,
            &RegisterParams {
                email: "mallory@test.com".to_string(),
                password: "correct-horse-battery".to_string(),
                name: "Mallory".to_string(),
            },
        )
        .await
        .expect("create user");

        let creds = serde_json::json!({
            "email": "mallory@test.com",
            "password": "correct-horse-battery"
        });

        // Correct password, but unverified -> no session.
        let denied = request.post("/api/auth/login").json(&creds).await;
        assert_eq!(
            denied.status_code(),
            401,
            "unverified account was issued a session with the correct password"
        );

        // Verifying the email lifts the gate; the same credentials now succeed.
        let user = users::Model::find_by_email(&ctx.db, "mallory@test.com")
            .await
            .expect("find user");
        let active: users::ActiveModel = user.into();
        active.verified(&ctx.db).await.expect("mark verified");

        let ok = request.post("/api/auth/login").json(&creds).await;
        assert_eq!(
            ok.status_code(),
            200,
            "verified account with correct password could not log in"
        );
    })
    .await;
}

/// Runs the operator's `user:status` task, the way it is run on a live instance.
async fn set_status(
    ctx: &loco_rs::app::AppContext,
    email: &str,
    status: &str,
) -> loco_rs::Result<()> {
    let vars = task::Vars::from_cli_args(vec![
        ("email".to_string(), email.to_string()),
        ("status".to_string(), status.to_string()),
    ]);
    run_task::<App>(ctx, Some(&"user:status".to_string()), &vars).await
}

/// The secret a proxy-mode instance in these tests shares with its proxy.
pub(super) const PROXY_SECRET: &str = "shared-between-the-proxy-and-castle";

/// Boots the next app in proxy mode, the way production runs. Dropped, later
/// boots are back in jwt mode even if the test panicked.
pub(super) struct ProxyMode;

impl ProxyMode {
    pub(super) fn on() -> Self {
        std::env::set_var("CASTLE_AUTH_MODE", "proxy");
        std::env::set_var("CASTLE_PROXY_SECRET", PROXY_SECRET);
        Self
    }
}

impl Drop for ProxyMode {
    fn drop(&mut self) {
        std::env::remove_var("CASTLE_AUTH_MODE");
        std::env::remove_var("CASTLE_PROXY_SECRET");
    }
}

/// The header oauth2-proxy puts on what it forwards: Basic, with the secret as
/// the password.
pub(super) fn from_proxy(secret: &str) -> (&'static str, String) {
    use base64::prelude::{Engine as _, BASE64_STANDARD};
    (
        "authorization",
        format!(
            "Basic {}",
            BASE64_STANDARD.encode(format!("someone:{secret}"))
        ),
    )
}

#[tokio::test]
#[serial]
async fn a_disabled_account_is_refused_at_once() {
    fresh_db();
    request::<App, _, _>(|request, ctx| async move {
        let f = seed(&ctx).await;
        let (h, v) = bearer(&f.client_token);
        let paths = [
            "/api/auth/current".to_string(),
            "/api/projects".to_string(),
            format!("/api/findings/{}", f.published_id),
        ];

        for path in &paths {
            let response = request.get(path).add_header(h, &v).await;
            assert_eq!(response.status_code(), 200, "{path} before disabling");
        }

        set_status(&ctx, "client@test.com", "disabled")
            .await
            .expect("disable");

        // The token is as valid as it was a moment ago; the account is not.
        for path in &paths {
            let response = request.get(path).add_header(h, &v).await;
            assert_eq!(response.status_code(), 403, "{path} after disabling");
        }
        let response = request
            .post(&format!("/api/findings/{}/comments", f.published_id))
            .add_header(h, &v)
            .json(&serde_json::json!({ "body": "still here" }))
            .await;
        assert_eq!(response.status_code(), 403);

        // Nobody else is touched.
        let (sh, sv) = bearer(&f.staff_token);
        let response = request.get("/api/auth/current").add_header(sh, &sv).await;
        assert_eq!(response.status_code(), 200);

        set_status(&ctx, "client@test.com", "active")
            .await
            .expect("enable");
        let response = request.get("/api/auth/current").add_header(h, &v).await;
        assert_eq!(response.status_code(), 200);
    })
    .await;
}

#[tokio::test]
#[serial]
async fn a_proxy_session_does_not_re_enable_a_disabled_account() {
    fresh_db();
    let _proxy = ProxyMode::on();
    request::<App, _, _>(|request, ctx| async move {
        let proxy = from_proxy(PROXY_SECRET);
        let current = || {
            request
                .get("/api/auth/current")
                .add_header(proxy.0, &proxy.1)
                .add_header("x-forwarded-email", "client@example.com")
                .add_header("x-forwarded-preferred-username", "client")
                .add_header("x-forwarded-groups", "castle-clients")
        };

        // The first request provisions the account, as the first sign-in does.
        assert_eq!(current().await.status_code(), 200);

        set_status(&ctx, "client@example.com", "disabled")
            .await
            .expect("disable");

        // oauth2-proxy still vouches for the session. Castle refuses it, and
        // being asked again must not flip the account back to active.
        assert_eq!(current().await.status_code(), 403);
        assert_eq!(current().await.status_code(), 403);
        let user = users::Model::find_by_email(&ctx.db, "client@example.com")
            .await
            .expect("find user");
        assert_eq!(user.status, UserStatus::Disabled);

        set_status(&ctx, "client@example.com", "active")
            .await
            .expect("enable");
        assert_eq!(current().await.status_code(), 200);
    })
    .await;
}

/// Memberships are management's to change, and a change decides what the member
/// can do from the next request on.
#[tokio::test]
#[serial]
async fn management_changes_and_removes_memberships() {
    fresh_db();
    request::<App, _, _>(|request, ctx| async move {
        let f = seed(&ctx).await;
        let (mh, mv) = bearer(&f.manager_token);
        let (sh, sv) = bearer(&f.staff_token);
        let project = format!("/api/projects/{}", f.project_id);
        let members = format!("{project}/members");
        let draft = format!("/api/findings/{}", f.draft_id);
        let as_client = serde_json::json!({ "role": "client" });

        let listed: Vec<serde_json::Value> = request.get(&members).add_header(mh, &mv).await.json();
        let staff = listed
            .iter()
            .find(|m| m["user"]["email"] == "staff@test.com")
            .expect("the staff member");
        let member = format!("{members}/{}", staff["id"]);

        for token in [&f.staff_token, &f.client_token, &f.outsider_token] {
            let (h, v) = bearer(token);
            let changed = request.put(&member).add_header(h, &v).json(&as_client);
            assert_eq!(changed.await.status_code(), 403);
            let removed = request.delete(&member).add_header(h, &v);
            assert_eq!(removed.await.status_code(), 403);
        }
        let read = request.get(&draft).add_header(sh, &sv).await;
        assert_eq!(read.status_code(), 200);

        // As a client of the project the same account reads what clients read.
        let changed = request
            .put(&member)
            .add_header(mh, &mv)
            .json(&as_client)
            .await;
        assert_eq!(changed.status_code(), 200);
        assert_eq!(changed.json::<serde_json::Value>()["role"], "client");
        let read = request.get(&draft).add_header(sh, &sv).await;
        assert_eq!(read.status_code(), 404);

        let changed = request
            .put(&member)
            .add_header(mh, &mv)
            .json(&serde_json::json!({ "role": "staff" }))
            .await;
        assert_eq!(changed.status_code(), 200);
        let read = request.get(&draft).add_header(sh, &sv).await;
        assert_eq!(read.status_code(), 200);

        // Management is not a capacity a membership can give.
        let minted = request
            .put(&member)
            .add_header(mh, &mv)
            .json(&serde_json::json!({ "role": "manager" }))
            .await;
        assert_eq!(minted.status_code(), 400);

        // A membership is reached through its own project only.
        let other: serde_json::Value = request
            .post("/api/projects")
            .add_header(mh, &mv)
            .json(&serde_json::json!({ "name": "Other" }))
            .await
            .json();
        let elsewhere = format!("/api/projects/{}/members/{}", other["id"], staff["id"]);
        let changed = request
            .put(&elsewhere)
            .add_header(mh, &mv)
            .json(&as_client)
            .await;
        assert_eq!(changed.status_code(), 404);
        let removed = request.delete(&elsewhere).add_header(mh, &mv).await;
        assert_eq!(removed.status_code(), 404);

        // Removed, the project is to them like any other they are not on.
        let removed = request.delete(&member).add_header(mh, &mv).await;
        assert_eq!(removed.status_code(), 200);
        for path in [&project, &draft] {
            let read = request.get(path).add_header(sh, &sv).await;
            assert_eq!(read.status_code(), 404, "{path}");
        }
        let removed = request.delete(&member).add_header(mh, &mv).await;
        assert_eq!(removed.status_code(), 404);
    })
    .await;
}

/// Identity headers are believed only from this instance's own proxy, which
/// proves itself with the secret the two share. Anything else that reaches the
/// port, whatever headers it sends, is nobody.
#[tokio::test]
#[serial]
async fn identity_headers_count_only_with_the_proxy_secret() {
    fresh_db();
    let _proxy = ProxyMode::on();
    request::<App, _, _>(|request, _ctx| async move {
        let as_manager = || {
            request
                .get("/api/projects")
                .add_header("x-forwarded-email", "intruder@example.com")
                .add_header("x-forwarded-groups", "castle-managers")
        };

        assert_eq!(as_manager().await.status_code(), 401, "no secret");
        let wrong = from_proxy("not-the-secret");
        let refused = as_manager().add_header(wrong.0, &wrong.1).await;
        assert_eq!(refused.status_code(), 401, "a wrong secret");
        let empty = from_proxy("");
        let refused = as_manager().add_header(empty.0, &empty.1).await;
        assert_eq!(refused.status_code(), 401, "an empty secret");
        let bare = as_manager()
            .add_header("authorization", format!("Bearer {PROXY_SECRET}"))
            .await;
        assert_eq!(
            bare.status_code(),
            401,
            "the secret outside the Basic scheme"
        );
        let right = from_proxy(PROXY_SECRET);
        let twice = as_manager()
            .add_header(right.0, &right.1)
            .add_header(wrong.0, &wrong.1)
            .await;
        assert_eq!(twice.status_code(), 401, "two Authorization headers");

        let through = as_manager().add_header(right.0, &right.1).await;
        assert_eq!(through.status_code(), 200, "through the proxy");

        // Nobody who was refused left an account behind.
        let (h, v) = (right.0, right.1.clone());
        let current: serde_json::Value = request
            .get("/api/auth/current")
            .add_header(h, &v)
            .add_header("x-forwarded-email", "intruder@example.com")
            .add_header("x-forwarded-groups", "castle-managers")
            .await
            .json();
        assert_eq!(
            current["id"], 1,
            "the first account is the one that came through"
        );
    })
    .await;
}

/// What the proxy says is read strictly. A header sent twice or an address
/// that is none makes the request nobody's; a group counts by its exact name.
#[tokio::test]
#[serial]
async fn the_identity_headers_are_read_strictly() {
    fresh_db();
    let _proxy = ProxyMode::on();
    request::<App, _, _>(|request, _ctx| async move {
        let proxy = from_proxy(PROXY_SECRET);
        let ask = || {
            request
                .get("/api/auth/current")
                .add_header(proxy.0, &proxy.1)
        };

        let twice = ask()
            .add_header("x-forwarded-email", "someone@example.com")
            .add_header("x-forwarded-email", "manager@example.com")
            .await;
        assert_eq!(twice.status_code(), 401, "two addresses");
        let groups_twice = ask()
            .add_header("x-forwarded-email", "someone@example.com")
            .add_header("x-forwarded-groups", "castle-clients")
            .add_header("x-forwarded-groups", "castle-managers")
            .await;
        assert_eq!(groups_twice.status_code(), 401, "two groups headers");
        for not_an_address in ["someone", "a b@example.com", "@example.com", ""] {
            let refused = ask().add_header("x-forwarded-email", not_an_address).await;
            assert_eq!(refused.status_code(), 401, "{not_an_address:?}");
        }

        let role_with = |groups: &'static str| async move {
            let who: serde_json::Value = ask()
                .add_header("x-forwarded-email", "someone@example.com")
                .add_header("x-forwarded-groups", groups)
                .await
                .json();
            who["role"].as_str().expect("a role").to_string()
        };
        for nearly in [
            "x/castle-managers",
            "/castle-managers",
            "a castle-managers",
            "castle-clients, castle-managers",
            "castle-managers;x",
            "castle-managers-x",
            "CASTLE-MANAGERS",
            "",
        ] {
            assert_eq!(role_with(nearly).await, "client", "{nearly:?}");
        }
        assert_eq!(role_with("castle-clients,castle-staff").await, "staff");
        assert_eq!(
            role_with("other,castle-managers,castle-staff").await,
            "manager"
        );

        // The first account is the only one: nothing above made another.
        let who: serde_json::Value = ask()
            .add_header("x-forwarded-email", "someone@example.com")
            .await
            .json();
        assert_eq!(who["id"], 1);
        assert_eq!(who["role"], "client", "no groups header is a client");
    })
    .await;
}

/// A name the account cannot carry is not taken, and the person signs in all
/// the same, every time.
#[tokio::test]
#[serial]
async fn a_name_that_does_not_fit_is_left_out() {
    fresh_db();
    let _proxy = ProxyMode::on();
    request::<App, _, _>(|request, _ctx| async move {
        let proxy = from_proxy(PROXY_SECRET);
        let (request, proxy) = (&request, &proxy);
        let named = |name: &'static str| async move {
            let answer = request
                .get("/api/auth/current")
                .add_header(proxy.0, &proxy.1)
                .add_header("x-forwarded-email", "someone@example.com")
                .add_header("x-forwarded-preferred-username", name)
                .await;
            assert_eq!(answer.status_code(), 200, "{name:?}");
            let who: serde_json::Value = answer.json();
            who["name"].as_str().expect("a name").to_string()
        };
        assert_eq!(named("x").await, "someone@example.com");
        assert_eq!(named("x").await, "someone@example.com", "and again");
        assert_eq!(named("  Someone  ").await, "Someone");
        assert_eq!(named("é").await, "Someone", "one character, two bytes");
        assert_eq!(named("").await, "Someone");
        assert_eq!(named("Zoë").await, "Zoë");
    })
    .await;
}

/// The session is a cookie, which a browser also sends with requests that
/// another site starts. So a request that changes something has to come from
/// the portal's own pages, by the browser's word.
#[tokio::test]
#[serial]
async fn a_changing_request_comes_from_the_portal() {
    fresh_db();
    let _proxy = ProxyMode::on();
    request::<App, _, _>(|request, ctx| async move {
        let proxy = from_proxy(PROXY_SECRET);
        let (request, proxy) = (&request, &proxy);
        let site = ctx.config.server.host.clone();
        let create = |browser: Vec<(&'static str, String)>| async move {
            let mut call = request
                .post("/api/projects")
                .add_header(proxy.0, &proxy.1)
                .add_header("x-forwarded-email", "manager@example.com")
                .add_header("x-forwarded-groups", "castle-managers");
            for (name, value) in browser {
                call = call.add_header(name, &value);
            }
            call.json(&serde_json::json!({ "name": "Engagement" }))
                .await
                .status_code()
        };
        let says = |relation: &str| vec![("sec-fetch-site", relation.to_string())];

        assert_eq!(create(says("cross-site")).await, 403);
        assert_eq!(create(says("same-site")).await, 403);
        assert_eq!(create(says("none")).await, 403);
        assert_eq!(create(Vec::new()).await, 403, "the browser says nothing");
        let elsewhere = vec![("origin", "https://elsewhere.example".to_string())];
        assert_eq!(
            create(elsewhere).await,
            403,
            "an older browser, another site"
        );
        let both = vec![
            ("sec-fetch-site", "cross-site".to_string()),
            ("origin", site.clone()),
        ];
        assert_eq!(create(both).await, 403, "the origin does not outvote it");

        assert_eq!(create(says("same-origin")).await, 200);
        assert_eq!(
            create(vec![("origin", site)]).await,
            200,
            "an older browser, this site"
        );

        // Reading is not changing: a link from elsewhere opens the portal.
        let read = request
            .get("/api/projects")
            .add_header(proxy.0, &proxy.1)
            .add_header("sec-fetch-site", "cross-site")
            .add_header("x-forwarded-email", "manager@example.com")
            .add_header("x-forwarded-groups", "castle-managers")
            .await;
        assert_eq!(read.status_code(), 200);
        let projects: Vec<serde_json::Value> = read.json();
        assert_eq!(projects.len(), 2, "only the two that came from the portal");
    })
    .await;
}

/// Staff capacity on a project takes the IdP's word and a manager's. An account
/// the IdP calls a client stays one whatever it is onboarded as, whichever of
/// the two happened first.
#[tokio::test]
#[serial]
async fn a_membership_does_not_outrank_the_idp() {
    fresh_db();
    let _proxy = ProxyMode::on();
    request::<App, _, _>(|request, _ctx| async move {
        let proxy = from_proxy(PROXY_SECRET);
        let get = |path: &str, email: &str, group: &str| {
            request
                .get(path)
                .add_header(proxy.0, &proxy.1)
                .add_header("x-forwarded-email", email)
                .add_header("x-forwarded-groups", group)
        };
        let post = |path: &str, email: &str, group: &str| {
            request
                .post(path)
                .add_header(proxy.0, &proxy.1)
                .add_header("sec-fetch-site", "same-origin")
                .add_header("x-forwarded-email", email)
                .add_header("x-forwarded-groups", group)
        };
        let manager = "manager@example.com";
        let finding = serde_json::json!({
            "title": "Draft",
            "description": "d",
            "technical_description": "t",
            "impact": "i",
            "recommendation": "r",
        });

        let project: serde_json::Value = post("/api/projects", manager, "castle-managers")
            .json(&serde_json::json!({ "name": "Engagement" }))
            .await
            .json();
        let members = format!("/api/projects/{}/members", project["id"]);
        let findings = format!("/api/projects/{}/findings", project["id"]);
        let draft: serde_json::Value = post(&findings, manager, "castle-managers")
            .json(&finding)
            .await
            .json();
        let draft = format!("/api/findings/{}", draft["id"]);

        // One account has signed in before it is onboarded, the other has not.
        let seen = get("/api/auth/current", "early@example.com", "castle-clients").await;
        assert_eq!(seen.status_code(), 200);

        for email in ["early@example.com", "late@example.com"] {
            let onboarded = post(&members, manager, "castle-managers")
                .json(&serde_json::json!({ "user_email": email, "role": "staff" }))
                .await;
            assert_eq!(onboarded.status_code(), 200, "onboarding {email}");

            let listed: Vec<serde_json::Value> =
                get(&findings, email, "castle-clients").await.json();
            assert!(listed.is_empty(), "{email} is shown a draft");
            let read = get(&draft, email, "castle-clients").await;
            assert_eq!(read.status_code(), 404, "{email} reads a draft");
            let written = post(&findings, email, "castle-clients")
                .json(&finding)
                .await;
            assert_eq!(written.status_code(), 403, "{email} writes a finding");
            let published = post(&format!("{draft}/publish"), email, "castle-clients").await;
            assert_eq!(published.status_code(), 404, "{email} publishes a draft");
            let commented = post(&format!("{draft}/comments"), email, "castle-clients")
                .json(&serde_json::json!({ "body": "hello" }))
                .await;
            assert_eq!(commented.status_code(), 404, "{email} comments on a draft");
        }

        // The member list says so too: staff by membership, a client by what
        // they may do.
        let listed: Vec<serde_json::Value> = get(&members, manager, "castle-managers").await.json();
        for email in ["early@example.com", "late@example.com"] {
            let row = listed
                .iter()
                .find(|m| m["user"]["email"] == email)
                .expect("listed");
            assert_eq!(row["role"], "staff", "{email}");
            assert_eq!(row["access"], "client", "{email}");
        }

        // The same membership does make staff of someone the IdP calls staff.
        let onboarded = post(&members, manager, "castle-managers")
            .json(&serde_json::json!({ "user_email": "tester@example.com", "role": "staff" }))
            .await;
        assert_eq!(onboarded.status_code(), 200);
        let read = get(&draft, "tester@example.com", "castle-staff").await;
        assert_eq!(read.status_code(), 200);
        let written = post(&findings, "tester@example.com", "castle-staff")
            .json(&finding)
            .await;
        assert_eq!(written.status_code(), 200);

        // For as long as the IdP says so: it is asked at every request, and the
        // membership stays what management made it.
        for (group, expected) in [("castle-clients", 403), ("castle-staff", 200)] {
            let written = post(&findings, "tester@example.com", group)
                .json(&finding)
                .await;
            assert_eq!(written.status_code(), expected, "as {group}");
        }
    })
    .await;
}

#[tokio::test]
#[serial]
async fn the_status_task_refuses_what_it_does_not_understand() {
    fresh_db();
    request::<App, _, _>(|_request, ctx| async move {
        seed(&ctx).await;

        assert!(set_status(&ctx, "client@test.com", "invited")
            .await
            .is_err());
        assert!(set_status(&ctx, "nobody@test.com", "disabled")
            .await
            .is_err());

        let user = users::Model::find_by_email(&ctx.db, "client@test.com")
            .await
            .expect("find user");
        assert_eq!(user.status, UserStatus::Active);
    })
    .await;
}
