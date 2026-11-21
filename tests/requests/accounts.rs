//! Accounts: the list management sees.

use castle::app::App;
use loco_rs::testing::prelude::*;
use serial_test::serial;

use super::authz::{bearer, fresh_db, seed};

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
