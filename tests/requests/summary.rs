//! What an instance says about itself to the host it runs on.

use castle::{
    app::App,
    models::_entities::{comments, findings, projects},
    models::findings::{FindingStatus, Severity},
    tasks::summary::Summary,
};
use chrono::{DateTime, FixedOffset, Utc};
use loco_rs::{boot::run_task, task, testing::prelude::*};
use sea_orm::{ActiveModelTrait, ActiveValue::Set, DatabaseConnection};
use serial_test::serial;

use super::authz::{fresh_db, make_user};

/// The summary of the instance seeded below. The overview's recognizers are
/// tested against this same file, so the three agree on what a summary is.
const FIXTURE: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/deploy/overview/fixtures/summary/taken/castle.json"
));

fn at(time: &str) -> DateTime<FixedOffset> {
    DateTime::parse_from_rfc3339(time).expect("a time")
}

async fn make_project(db: &DatabaseConnection, name: &str, created_by: i64) -> i64 {
    projects::ActiveModel {
        name: Set(name.to_string()),
        description: Set(Some("Planted project description".to_string())),
        created_by: Set(created_by),
        ..Default::default()
    }
    .insert(db)
    .await
    .expect("create project")
    .id
}

async fn make_finding(
    db: &DatabaseConnection,
    project_id: i64,
    author_id: i64,
    (status, severity): (FindingStatus, Severity),
    updated_at: &str,
) -> i64 {
    findings::ActiveModel {
        project_id: Set(project_id),
        author_id: Set(author_id),
        title: Set("Planted finding title".to_string()),
        description: Set("Planted description".to_string()),
        technical_description: Set("Planted technical description".to_string()),
        impact: Set("Planted impact".to_string()),
        recommendation: Set("Planted recommendation".to_string()),
        status: Set(status),
        severity: Set(severity),
        updated_at: Set(at(updated_at)),
        ..Default::default()
    }
    .insert(db)
    .await
    .expect("create finding")
    .id
}

async fn make_comment(db: &DatabaseConnection, finding_id: i64, user_id: i64, created_at: &str) {
    comments::ActiveModel {
        finding_id: Set(finding_id),
        user_id: Set(user_id),
        body: Set("Planted comment body".to_string()),
        created_at: Set(at(created_at)),
        ..Default::default()
    }
    .insert(db)
    .await
    .expect("create comment");
}

fn now() -> DateTime<Utc> {
    at("2026-11-20T12:00:00Z").with_timezone(&Utc)
}

#[tokio::test]
#[serial]
async fn a_summary_is_ids_counts_and_times() {
    fresh_db();
    request::<App, _, _>(|_request, ctx| async move {
        let db = &ctx.db;
        let manager = make_user(db, "manager@test.com", "Planted Manager", "manager").await;
        let staff = make_user(db, "staff@test.com", "Planted Staff", "staff").await;
        let client = make_user(db, "client@test.com", "Planted Client", "client").await;

        let busy = make_project(db, "Planted project name", manager.id).await;
        let empty = make_project(db, "Planted empty project", manager.id).await;
        let quiet = make_project(db, "Planted quiet project", manager.id).await;

        let high = make_finding(
            db,
            busy,
            staff.id,
            (FindingStatus::Published, Severity::High),
            "2026-11-10T09:00:00Z",
        )
        .await;
        let medium = make_finding(
            db,
            busy,
            staff.id,
            (FindingStatus::Published, Severity::Medium),
            "2026-11-19T08:30:00.750Z",
        )
        .await;
        let draft = make_finding(
            db,
            busy,
            staff.id,
            (FindingStatus::Draft, Severity::Extreme),
            "2026-11-18T10:00:00Z",
        )
        .await;
        let old = make_finding(
            db,
            quiet,
            staff.id,
            (FindingStatus::Published, Severity::Low),
            "2026-10-01T07:00:00Z",
        )
        .await;

        make_comment(db, high, client.id, "2026-11-19T15:45:10Z").await;
        make_comment(db, high, staff.id, "2026-11-19T16:00:00+00:00").await;
        // Before the window: it counts for nothing but the last activity.
        make_comment(db, medium, client.id, "2026-11-12T10:00:00Z").await;
        make_comment(db, draft, manager.id, "2026-11-20T12:59:59+01:00").await;
        make_comment(db, old, client.id, "2026-10-05T07:00:00Z").await;

        let summary = Summary::of(db, now(), 7).await.expect("summary");
        let printed = serde_json::to_string(&summary).expect("json");
        let expected: serde_json::Value = serde_json::from_str(FIXTURE).expect("fixture");
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&printed).expect("json"),
            expected,
            "the instance says:\n{printed}"
        );
        assert_eq!(expected["projects"][1]["id"], empty);

        // Nothing anyone typed is in it.
        for typed in ["Planted", "planted", "test.com"] {
            assert!(!printed.contains(typed), "the summary carries `{typed}`");
        }
    })
    .await;
}

#[tokio::test]
#[serial]
async fn the_counts_cover_what_the_list_leaves_out() {
    fresh_db();
    request::<App, _, _>(|_request, ctx| async move {
        let db = &ctx.db;
        let staff = make_user(db, "staff@test.com", "Staff", "staff").await;
        let project = make_project(db, "Engagement", staff.id).await;
        let finding = make_finding(
            db,
            project,
            staff.id,
            (FindingStatus::Published, Severity::Low),
            "2026-11-01T00:00:00Z",
        )
        .await;
        for minute in 0..60 {
            make_comment(
                db,
                finding,
                staff.id,
                &format!("2026-11-20T10:{minute:02}:00Z"),
            )
            .await;
        }

        let summary =
            serde_json::to_value(Summary::of(db, now(), 7).await.expect("summary")).expect("json");
        assert_eq!(summary["projects"][0]["comments"]["staff"], 60);
        let listed = summary["activity"].as_array().expect("a list");
        assert_eq!(listed.len(), 50);
        assert_eq!(listed[0]["at"], "2026-11-20T10:59:00Z");
        assert_eq!(listed[49]["at"], "2026-11-20T10:10:00Z");
    })
    .await;
}

#[tokio::test]
#[serial]
async fn the_summary_task_refuses_what_it_does_not_understand() {
    fresh_db();
    request::<App, _, _>(|_request, ctx| async move {
        // The tokens after the task's name, as the command line hands them over.
        let run = |args: Vec<&str>| {
            let vars = task::Vars::from_args(args.into_iter().map(str::to_string).collect());
            let ctx = ctx.clone();
            async move { run_task::<App>(&ctx, Some(&"summary".to_string()), &vars).await }
        };

        assert!(run(vec![]).await.is_ok());
        assert!(run(vec!["days:30"]).await.is_ok());
        assert!(run(vec!["days:90"]).await.is_ok());
        for refused in [
            vec!["days:0"],
            vec!["days:91"],
            vec!["days:a week"],
            vec!["days:-7"],
            vec!["days:+7"],
            vec!["days:"],
            vec!["days=30"],
            vec!["30"],
            vec!["--days", "30"],
            vec!["detail:titles"],
            vec!["days:7", "detail:titles"],
            vec!["days:7", "days:30"],
            vec!["days:7", ""],
        ] {
            assert!(run(refused.clone()).await.is_err(), "took {refused:?}");
        }
    })
    .await;
}

#[tokio::test]
#[serial]
async fn an_instance_with_more_projects_than_a_summary_carries_says_so() {
    fresh_db();
    request::<App, _, _>(|_request, ctx| async move {
        let db = &ctx.db;
        let manager = make_user(db, "manager@test.com", "Manager", "manager").await;
        for at in 1..=500 {
            make_project(db, &format!("Engagement {at}"), manager.id).await;
        }
        let summary =
            serde_json::to_value(Summary::of(db, now(), 7).await.expect("summary")).expect("json");
        assert_eq!(summary["projects"].as_array().expect("a list").len(), 500);

        make_project(db, "One more", manager.id).await;
        let refused = Summary::of(db, now(), 7)
            .await
            .expect_err("more than it carries");
        assert!(refused.to_string().contains("501 projects"), "{refused}");
    })
    .await;
}
