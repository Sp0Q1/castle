//! The seed fixture: what `cargo run -- db seed` loads, and what the README
//! tells a new developer to log in with.
//!
//! Nothing else in the suite reads `src/fixtures`, which is how the seed sat
//! broken for two months. `users.status` became a typed value, the fixture had
//! never stated one, and loading it failed with "missing field `status`" — a
//! documented command failing for everyone who tried it, with CI green
//! throughout.

use castle::{app::App, models::_entities::users, models::users::UserStatus};
use loco_rs::{app::Hooks, testing::prelude::*};
use sea_orm::{EntityTrait, QueryOrder};
use serial_test::serial;
use std::path::Path;

#[tokio::test]
#[serial]
async fn the_demo_accounts_seed_and_can_log_in() {
    super::authz::fresh_db();
    request::<App, _, _>(|request, ctx| async move {
        App::seed(&ctx, Path::new("src/fixtures"))
            .await
            .expect("the seed fixture loads");

        let seeded = users::Entity::find()
            .order_by_asc(users::Column::Id)
            .all(&ctx.db)
            .await
            .expect("list users");
        let accounts: Vec<_> = seeded
            .iter()
            .map(|u| (u.email.as_str(), u.role.as_str(), u.status))
            .collect();
        assert_eq!(
            accounts,
            [
                ("manager@example.com", "manager", UserStatus::Active),
                ("staff@example.com", "staff", UserStatus::Active),
                ("client@example.com", "client", UserStatus::Active),
            ]
        );

        // The password the README documents for all three.
        for (email, _, _) in &accounts {
            let res = request
                .post("/api/auth/login")
                .json(&serde_json::json!({ "email": email, "password": "12341234" }))
                .await;
            assert_eq!(res.status_code(), 200, "{email} can log in");
        }
    })
    .await;
}
