//! Accounts: the list management sees, and what becomes of someone who leaves.

use castle::{
    app::App,
    models::_entities::{findings, users},
    models::users::UserStatus,
};
use loco_rs::{boot::run_task, task, testing::prelude::*};
use sea_orm::{DatabaseBackend, EntityTrait, ModelTrait, PaginatorTrait};
use serial_test::serial;

use super::authz::{bearer, fresh_db, seed};

/// Runs the operator's `user:anonymize` task, the way it is run on a live instance.
async fn anonymize(ctx: &loco_rs::app::AppContext, email: &str) -> loco_rs::Result<()> {
    let vars = task::Vars::from_cli_args(vec![("email".to_string(), email.to_string())]);
    run_task::<App>(ctx, Some(&"user:anonymize".to_string()), &vars).await
}

#[tokio::test]
#[serial]
async fn management_sees_everyone_with_an_account() {
    fresh_db();
    request::<App, _, _>(|request, ctx| async move {
        let f = seed(&ctx).await;

        let anonymous = request.get("/api/users").await;
        assert_eq!(anonymous.status_code(), 401);
        for token in [&f.staff_token, &f.client_token, &f.outsider_token] {
            let (h, v) = bearer(token);
            let refused = request.get("/api/users").add_header(h, &v).await;
            assert_eq!(refused.status_code(), 403);
        }

        let (h, v) = bearer(&f.manager_token);
        let response = request.get("/api/users").add_header(h, &v).await;
        assert_eq!(response.status_code(), 200);
        let text = response.text();
        for secret in ["password", "api_key", "token"] {
            assert!(!text.contains(secret), "the list carries `{secret}`");
        }

        let people: Vec<serde_json::Value> = response.json();
        let emails: Vec<&str> = people.iter().filter_map(|p| p["email"].as_str()).collect();
        assert_eq!(
            emails,
            [
                "client@test.com",
                "manager@test.com",
                "outsider@test.com",
                "staff@test.com"
            ]
        );
        let person = |email: &str| {
            people
                .iter()
                .find(|p| p["email"] == email)
                .expect("listed")
                .clone()
        };
        let staff = person("staff@test.com");
        assert_eq!(staff["role"], "staff");
        assert_eq!(staff["status"], "active");
        assert_eq!(
            staff["projects"],
            serde_json::json!([{ "id": f.project_id, "name": "Engagement", "role": "staff" }])
        );
        assert_eq!(person("client@test.com")["projects"][0]["role"], "client");
        assert_eq!(
            person("outsider@test.com")["projects"],
            serde_json::json!([])
        );
    })
    .await;
}

/// Someone who leaves is anonymized, not deleted: what they wrote stays in the
/// report under a placeholder, and nothing else of them does.
#[tokio::test]
#[serial]
async fn an_anonymized_person_leaves_their_words_and_nothing_else() {
    fresh_db();
    request::<App, _, _>(|request, ctx| async move {
        let f = seed(&ctx).await;
        let (ch, cv) = bearer(&f.client_token);
        let (mh, mv) = bearer(&f.manager_token);
        let finding = format!("/api/findings/{}", f.published_id);
        let said = request
            .post(&format!("{finding}/comments"))
            .add_header(ch, &cv)
            .json(&serde_json::json!({ "body": "We fixed this on Monday." }))
            .await;
        assert_eq!(said.status_code(), 200);

        anonymize(&ctx, "client@test.com").await.expect("anonymize");

        // The session they still hold is worth nothing.
        let current = request.get("/api/auth/current").add_header(ch, &cv).await;
        assert_eq!(current.status_code(), 403);

        // The comment is still part of the finding, under the placeholder.
        let detail: serde_json::Value = request.get(&finding).add_header(mh, &mv).await.json();
        assert_eq!(detail["comments"][0]["body"], "We fixed this on Monday.");
        assert_eq!(detail["comments"][0]["author"]["name"], "Former user");
        assert!(!detail.to_string().contains("client@test.com"));

        // They are on no project any more, and no list names them.
        let members = request
            .get(&format!("/api/projects/{}/members", f.project_id))
            .add_header(mh, &mv)
            .await
            .text();
        assert!(!members.contains("client@test.com") && !members.contains("Former user"));
        let people: Vec<serde_json::Value> =
            request.get("/api/users").add_header(mh, &mv).await.json();
        assert!(people.iter().all(|p| p["email"] != "client@test.com"));
        let former = people
            .iter()
            .find(|p| p["name"] == "Former user")
            .expect("the placeholder is listed");
        assert_eq!(former["status"], "disabled");
        assert_eq!(former["projects"], serde_json::json!([]));

        // Nothing usable is left on the row either.
        let row = users::Entity::find_by_id(former["id"].as_i64().expect("id"))
            .one(&ctx.db)
            .await
            .expect("query")
            .expect("row");
        assert_eq!(row.status, UserStatus::Disabled);
        assert!(row.email.ends_with("@removed.invalid"));
        assert!(row.reset_token.is_none() && row.magic_link_token.is_none());

        // There is nobody by that address to anonymize a second time, and the
        // task does nothing without one.
        assert!(anonymize(&ctx, "client@test.com").await.is_err());
        let nobody = task::Vars::from_cli_args(vec![]);
        let refused = run_task::<App>(&ctx, Some(&"user:anonymize".to_string()), &nobody).await;
        assert!(refused.is_err());
    })
    .await;
}

/// The database itself will not delete a user that anything points at. SQLite
/// cannot be given that rule (see the migration); deployments and CI run
/// Postgres, and that is where this is asked.
#[tokio::test]
#[serial]
async fn the_database_refuses_to_delete_a_user_with_content() {
    fresh_db();
    request::<App, _, _>(|_request, ctx| async move {
        if ctx.db.get_database_backend() != DatabaseBackend::Postgres {
            return;
        }
        seed(&ctx).await;
        let written = findings::Entity::find()
            .count(&ctx.db)
            .await
            .expect("count");

        let staff = users::Model::find_by_email(&ctx.db, "staff@test.com")
            .await
            .expect("the author of both findings");
        assert!(staff.delete(&ctx.db).await.is_err());
        let manager = users::Model::find_by_email(&ctx.db, "manager@test.com")
            .await
            .expect("who opened the project");
        assert!(manager.delete(&ctx.db).await.is_err());
        let client = users::Model::find_by_email(&ctx.db, "client@test.com")
            .await
            .expect("a member");
        assert!(client.delete(&ctx.db).await.is_err());

        let kept = findings::Entity::find()
            .count(&ctx.db)
            .await
            .expect("count");
        assert_eq!(kept, written);

        // An account nothing points at can go.
        let outsider = users::Model::find_by_email(&ctx.db, "outsider@test.com")
            .await
            .expect("on no project");
        assert!(outsider.delete(&ctx.db).await.is_ok());
    })
    .await;
}
