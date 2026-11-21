//! What each kind of person gets from each address the server exposes: one
//! table, and a test that calls every address as everyone. An address that is
//! not in the table fails the test, so nothing is served without a stated rule.

use std::collections::BTreeSet;
use std::sync::atomic::{AtomicU32, Ordering};

use axum::body::Bytes;
use axum::http::Method;
use castle::{
    app::App,
    models::_entities::{project_members, users},
    models::findings::FindingStatus,
    models::project_members::MemberRole,
    models::users::UserStatus,
};
use loco_rs::{app::Hooks, testing::prelude::*};
use sea_orm::{ActiveModelTrait, ActiveValue::Set, ColumnTrait, EntityTrait, QueryFilter};
use serial_test::serial;

use super::authz::{
    bearer, fresh_db, from_proxy, id_of, make_finding, make_user, seed, ProxyMode, PROXY_SECRET,
};

/// Who asks, in the order of every `expect` below.
const ACTORS: [&str; 7] = [
    "nobody",
    "a manager",
    "staff on the project",
    "a client on the project",
    "a client on no project",
    "staff on no project",
    "a disabled account",
];

/// How each of them shows who they are. With castle's own sign-in that is a
/// token; in a deployment it is what the proxy sends for them: its secret, the
/// address and the group Keycloak has them in.
#[derive(Clone, Copy)]
enum SignIn {
    Jwt,
    Proxy,
}

/// The address and Keycloak group of each of [`ACTORS`], nobody excepted.
const PEOPLE: [Option<(&str, &str)>; 7] = [
    None,
    Some(("manager@test.com", "castle-managers")),
    Some(("staff@test.com", "castle-staff")),
    Some(("client@test.com", "castle-clients")),
    Some(("outsider@test.com", "castle-clients")),
    Some(("elsewhere@test.com", "castle-staff")),
    Some(("gone@test.com", "castle-staff")),
];

/// What the address is asked about. It is made anew for every call, so one
/// person's success leaves nothing changed for the next.
#[derive(Clone, Copy)]
enum On {
    Nothing,
    Project,
    Draft,
    Published,
    Member,
    Upload,
}

struct Rule {
    method: Method,
    route: &'static str,
    on: On,
    body: Option<&'static str>,
    expect: [u16; 7],
}

const fn rule(
    method: Method,
    route: &'static str,
    on: On,
    body: Option<&'static str>,
    expect: [u16; 7],
) -> Rule {
    Rule {
        method,
        route,
        on,
        body,
        expect,
    }
}

const FINDING: &str = r#"{"title":"T","description":"d","technical_description":"t","impact":"i","recommendation":"r"}"#;
const UPLOAD: &str = "multipart";

/// Anyone signed in, whatever they are.
const SIGNED_IN: [u16; 7] = [401, 200, 200, 200, 200, 200, 403];
/// Management only.
const MANAGEMENT: [u16; 7] = [401, 200, 403, 403, 403, 403, 403];
/// Whoever can open the project; to everyone else it does not exist.
const ON_THE_PROJECT: [u16; 7] = [401, 200, 200, 200, 404, 404, 403];
/// Project staff; a draft does not exist for a client.
const STAFF_DRAFT: [u16; 7] = [401, 200, 200, 404, 404, 404, 403];
/// Project staff; a client knows the published finding and may not change it.
const STAFF_PUBLISHED: [u16; 7] = [401, 200, 200, 403, 404, 404, 403];

const RULES: &[Rule] = &[
    rule(
        Method::GET,
        "/api/auth/current",
        On::Nothing,
        None,
        SIGNED_IN,
    ),
    rule(Method::GET, "/api/users", On::Nothing, None, MANAGEMENT),
    rule(
        Method::POST,
        "/api/projects",
        On::Nothing,
        Some(r#"{"name":"P"}"#),
        MANAGEMENT,
    ),
    rule(Method::GET, "/api/projects", On::Nothing, None, SIGNED_IN),
    rule(
        Method::GET,
        "/api/projects/{project_id}",
        On::Project,
        None,
        ON_THE_PROJECT,
    ),
    rule(
        Method::GET,
        "/api/projects/{project_id}/members",
        On::Project,
        None,
        ON_THE_PROJECT,
    ),
    rule(
        Method::GET,
        "/api/projects/{project_id}/managers",
        On::Project,
        None,
        ON_THE_PROJECT,
    ),
    rule(
        Method::POST,
        "/api/projects/{project_id}/members",
        On::Project,
        Some(r#"{"user_email":"new{n}@test.com","role":"client"}"#),
        MANAGEMENT,
    ),
    rule(
        Method::PUT,
        "/api/projects/{project_id}/members/{member_id}",
        On::Member,
        Some(r#"{"role":"staff"}"#),
        MANAGEMENT,
    ),
    rule(
        Method::DELETE,
        "/api/projects/{project_id}/members/{member_id}",
        On::Member,
        None,
        MANAGEMENT,
    ),
    rule(
        Method::POST,
        "/api/projects/{project_id}/findings",
        On::Project,
        Some(FINDING),
        [401, 200, 200, 403, 404, 404, 403],
    ),
    rule(
        Method::GET,
        "/api/projects/{project_id}/findings",
        On::Project,
        None,
        ON_THE_PROJECT,
    ),
    rule(
        Method::GET,
        "/api/findings/{finding_id}",
        On::Draft,
        None,
        STAFF_DRAFT,
    ),
    rule(
        Method::GET,
        "/api/findings/{finding_id}",
        On::Published,
        None,
        ON_THE_PROJECT,
    ),
    rule(
        Method::PUT,
        "/api/findings/{finding_id}",
        On::Draft,
        Some(r#"{"title":"U"}"#),
        STAFF_DRAFT,
    ),
    rule(
        Method::PUT,
        "/api/findings/{finding_id}",
        On::Published,
        Some(r#"{"title":"U"}"#),
        STAFF_PUBLISHED,
    ),
    rule(
        Method::DELETE,
        "/api/findings/{finding_id}",
        On::Draft,
        None,
        STAFF_DRAFT,
    ),
    rule(
        Method::DELETE,
        "/api/findings/{finding_id}",
        On::Published,
        None,
        STAFF_PUBLISHED,
    ),
    rule(
        Method::POST,
        "/api/findings/{finding_id}/publish",
        On::Draft,
        None,
        STAFF_DRAFT,
    ),
    rule(
        Method::POST,
        "/api/findings/{finding_id}/publish",
        On::Published,
        None,
        STAFF_PUBLISHED,
    ),
    rule(
        Method::POST,
        "/api/findings/{finding_id}/unpublish",
        On::Draft,
        None,
        STAFF_DRAFT,
    ),
    rule(
        Method::POST,
        "/api/findings/{finding_id}/unpublish",
        On::Published,
        None,
        STAFF_PUBLISHED,
    ),
    rule(
        Method::GET,
        "/api/findings/{finding_id}/comments",
        On::Draft,
        None,
        STAFF_DRAFT,
    ),
    rule(
        Method::GET,
        "/api/findings/{finding_id}/comments",
        On::Published,
        None,
        ON_THE_PROJECT,
    ),
    rule(
        Method::POST,
        "/api/findings/{finding_id}/comments",
        On::Draft,
        Some(r#"{"body":"c"}"#),
        STAFF_DRAFT,
    ),
    rule(
        Method::POST,
        "/api/findings/{finding_id}/comments",
        On::Published,
        Some(r#"{"body":"c"}"#),
        ON_THE_PROJECT,
    ),
    rule(
        Method::POST,
        "/api/uploads",
        On::Nothing,
        Some(UPLOAD),
        SIGNED_IN,
    ),
    rule(
        Method::GET,
        "/api/uploads/{name}",
        On::Upload,
        None,
        SIGNED_IN,
    ),
];

/// Addresses that ask for no session, each with the reason. The sign-in
/// endpoints exist in jwt mode only; `PROXY_OPEN` is what a deployment has.
const PROXY_OPEN: &[(&str, &str, &str)] = &[
    (
        "GET",
        "/_ping",
        "liveness, behind the proxy like everything else",
    ),
    (
        "GET",
        "/_health",
        "liveness, behind the proxy like everything else",
    ),
    (
        "GET",
        "/_readiness",
        "liveness, behind the proxy like everything else",
    ),
    (
        "GET",
        "/api/auth/mode",
        "the page asks it before anyone has signed in",
    ),
    (
        "GET",
        "/api/brand/logo",
        "the logo is on the sign-in page too",
    ),
];
const JWT_SIGN_IN: &[(&str, &str)] = &[
    ("POST", "/api/auth/register"),
    ("POST", "/api/auth/login"),
    ("POST", "/api/auth/forgot"),
    ("POST", "/api/auth/reset"),
    ("POST", "/api/auth/magic-link"),
    ("POST", "/api/auth/resend-verification-mail"),
    ("GET", "/api/auth/verify/{token}"),
    ("GET", "/api/auth/magic-link/{token}"),
];

fn served(ctx: &loco_rs::app::AppContext) -> BTreeSet<(String, String)> {
    App::routes(ctx)
        .collect()
        .iter()
        .flat_map(|route| {
            route
                .actions
                .iter()
                .map(|action| (action.to_string(), route.uri.clone()))
        })
        .collect()
}

fn stated(with_sign_in: bool) -> BTreeSet<(String, String)> {
    let rules = RULES
        .iter()
        .map(|r| (r.method.to_string(), r.route.to_string()));
    let open = PROXY_OPEN
        .iter()
        .map(|(method, route, _)| ((*method).to_string(), (*route).to_string()));
    let sign_in = JWT_SIGN_IN
        .iter()
        .filter(|_| with_sign_in)
        .map(|(method, route)| ((*method).to_string(), (*route).to_string()));
    rules.chain(open).chain(sign_in).collect()
}

fn assert_stated(served: &BTreeSet<(String, String)>, stated: &BTreeSet<(String, String)>) {
    let unstated: Vec<_> = served.difference(stated).collect();
    let unserved: Vec<_> = stated.difference(served).collect();
    assert!(
        unstated.is_empty() && unserved.is_empty(),
        "served without a stated rule: {unstated:?}; stated but not served: {unserved:?}"
    );
}

#[tokio::test]
#[serial]
async fn every_address_answers_each_kind_of_person_as_stated() {
    fresh_db();
    everyone_asks_everything(SignIn::Jwt).await;
}

/// The same table behind the proxy, which is how a deployment runs: who asks
/// is what the proxy says, and nothing is believed without its secret.
#[tokio::test]
#[serial]
async fn the_same_holds_behind_the_proxy() {
    fresh_db();
    let _proxy = ProxyMode::on();
    everyone_asks_everything(SignIn::Proxy).await;
}

async fn everyone_asks_everything(sign_in: SignIn) {
    request::<App, _, _>(|request, ctx| async move {
        assert_stated(&served(&ctx), &stated(matches!(sign_in, SignIn::Jwt)));

        let db = &ctx.db;
        let f = seed(&ctx).await;
        let jwt = ctx.config.get_jwt_config().expect("jwt config");
        let token = |user: &users::Model| {
            user.generate_jwt(&jwt.secret, jwt.expiration)
                .expect("generate jwt")
        };
        let other_staff = make_user(db, "elsewhere@test.com", "Elsewhere", "staff").await;
        let gone = make_user(db, "gone@test.com", "Gone", "staff").await;
        project_members::ActiveModel {
            project_id: Set(f.project_id),
            user_id: Set(gone.id),
            role: Set(MemberRole::Staff),
            ..Default::default()
        }
        .insert(db)
        .await
        .expect("put the account on the project");
        let gone_token = token(&gone);
        let mut gone: users::ActiveModel = gone.into();
        gone.status = Set(UserStatus::Disabled);
        gone.update(db).await.expect("disable the account");
        let tokens: [Option<String>; 7] = [
            None,
            Some(f.manager_token.clone()),
            Some(f.staff_token.clone()),
            Some(f.client_token.clone()),
            Some(f.outsider_token.clone()),
            Some(token(&other_staff)),
            Some(gone_token),
        ];
        let shows = |who: usize| -> Vec<(&'static str, String)> {
            match (sign_in, &tokens[who], PEOPLE[who]) {
                (SignIn::Jwt, Some(token), _) => vec![bearer(token)],
                (SignIn::Proxy, _, Some((email, group))) => vec![
                    from_proxy(PROXY_SECRET),
                    ("sec-fetch-site", "same-origin".to_string()),
                    ("x-forwarded-user", id_of(email)),
                    ("x-forwarded-email", email.to_string()),
                    ("x-forwarded-groups", group.to_string()),
                ],
                _ => Vec::new(),
            }
        };

        let staff = users::Model::find_by_email(db, "staff@test.com")
            .await
            .expect("staff");
        let spare = make_user(db, "spare@test.com", "Spare", "client").await;
        let image = [
            b"--b\r\nContent-Disposition: form-data; name=\"file\"; filename=\"x.png\"\r\nContent-Type: image/png\r\n\r\n".as_slice(),
            b"\x89PNG\r\n\x1a\n not much of a picture",
            b"\r\n--b--\r\n",
        ]
        .concat();
        let mut upload = request.post("/api/uploads");
        for (name, value) in shows(1) {
            upload = upload.add_header(name, &value);
        }
        let uploaded: serde_json::Value = upload
            .content_type("multipart/form-data; boundary=b")
            .bytes(Bytes::from(image.clone()))
            .await
            .json();
        let upload_name = uploaded["url"]
            .as_str()
            .and_then(|url| url.rsplit('/').next())
            .expect("an uploaded file")
            .to_string();

        static FRESH: AtomicU32 = AtomicU32::new(0);
        let mut wrong = Vec::new();
        for rule in RULES {
            for (who, expected) in rule.expect.iter().enumerate() {
                let n = FRESH.fetch_add(1, Ordering::Relaxed);
                let mut path = rule.route.replace("{project_id}", &f.project_id.to_string());
                match rule.on {
                    On::Nothing | On::Project => {}
                    On::Draft | On::Published => {
                        let status = if matches!(rule.on, On::Draft) {
                            FindingStatus::Draft
                        } else {
                            FindingStatus::Published
                        };
                        let finding =
                            make_finding(db, f.project_id, staff.id, "Target", status).await;
                        path = path.replace("{finding_id}", &finding.id.to_string());
                    }
                    On::Member => {
                        let existing = project_members::Entity::find()
                            .filter(project_members::Column::ProjectId.eq(f.project_id))
                            .filter(project_members::Column::UserId.eq(spare.id))
                            .one(db)
                            .await
                            .expect("query");
                        let member = match existing {
                            Some(member) => member,
                            None => project_members::ActiveModel {
                                project_id: Set(f.project_id),
                                user_id: Set(spare.id),
                                role: Set(MemberRole::Client),
                                ..Default::default()
                            }
                            .insert(db)
                            .await
                            .expect("a membership to act on"),
                        };
                        path = path.replace("{member_id}", &member.id.to_string());
                    }
                    On::Upload => path = path.replace("{name}", &upload_name),
                }

                let mut call = request.method(rule.method.clone(), &path);
                for (name, value) in shows(who) {
                    call = call.add_header(name, &value);
                }
                call = match rule.body {
                    Some(UPLOAD) => call
                        .content_type("multipart/form-data; boundary=b")
                        .bytes(Bytes::from(image.clone())),
                    Some(body) => call
                        .content_type("application/json")
                        .bytes(Bytes::from(body.replace("{n}", &n.to_string()))),
                    None => call,
                };
                let got = call.await.status_code().as_u16();
                if got != *expected {
                    wrong.push(format!(
                        "{} {} ({}) as {}: {got}, stated {expected}",
                        rule.method,
                        rule.route,
                        match rule.on {
                            On::Draft => "a draft",
                            On::Published => "published",
                            _ => "-",
                        },
                        ACTORS[who]
                    ));
                }
            }
        }
        assert!(wrong.is_empty(), "{} answers differ from the table:\n{}", wrong.len(), wrong.join("\n"));
    })
    .await;
}

/// The page is sent what the person may do, so it never works the rule out
/// again: each project carries the reader's access, and each member row what
/// that member may do.
#[tokio::test]
#[serial]
async fn a_project_tells_the_reader_what_they_may_do() {
    fresh_db();
    request::<App, _, _>(|request, ctx| async move {
        let f = seed(&ctx).await;
        let project = format!("/api/projects/{}", f.project_id);
        for (token, expected) in [
            (&f.manager_token, "staff"),
            (&f.staff_token, "staff"),
            (&f.client_token, "client"),
        ] {
            let (h, v) = bearer(token);
            let one: serde_json::Value = request.get(&project).add_header(h, &v).await.json();
            assert_eq!(one["access"], expected);
            let all: Vec<serde_json::Value> =
                request.get("/api/projects").add_header(h, &v).await.json();
            assert_eq!(all.len(), 1);
            assert_eq!(all[0]["access"], expected);
        }

        let (h, v) = bearer(&f.manager_token);
        let members: Vec<serde_json::Value> = request
            .get(&format!("{project}/members"))
            .add_header(h, &v)
            .await
            .json();
        let access_of = |email: &str| {
            members
                .iter()
                .find(|m| m["user"]["email"] == email)
                .map(|m| (m["role"].clone(), m["access"].clone()))
                .expect("listed")
        };
        assert_eq!(
            access_of("staff@test.com"),
            ("staff".into(), "staff".into())
        );
        assert_eq!(
            access_of("client@test.com"),
            ("client".into(), "client".into())
        );
    })
    .await;
}

/// A deployment runs in proxy mode, and there the sign-in endpoints of jwt
/// mode are not served at all.
#[tokio::test]
#[serial]
async fn a_deployment_serves_no_sign_in_of_its_own() {
    fresh_db();
    let _proxy = ProxyMode::on();
    request::<App, _, _>(|_request, ctx| async move {
        assert_stated(&served(&ctx), &stated(false));
    })
    .await;
}
