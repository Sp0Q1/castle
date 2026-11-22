use std::collections::HashMap;

use chrono::{DateTime, Duration, SubsecRound, Utc};
use loco_rs::prelude::*;
use sea_orm::{AccessMode, DerivePartialModel, IsolationLevel, QueryOrder, TransactionTrait};
use serde::Serialize;

use crate::models::{
    _entities::{comments, findings, projects, users},
    findings::{FindingStatus, Severity},
    users::UserRole,
};

/// How many of the newest comments are listed one by one. The counts cover
/// every comment in the window.
const ACTIVITY_LIMIT: usize = 50;
/// How many projects a summary carries. The host takes no summary that has
/// more (`deploy/compose/summary.jq`), so an instance with more says so.
const MAX_PROJECTS: usize = 500;
const DEFAULT_WINDOW_DAYS: u32 = 7;
const MAX_WINDOW_DAYS: u32 = 90;

/// What an instance says about itself to the host it runs on: ids, counts,
/// times and words from a closed set, enough to say where something is new
/// and to link to it. The rows it is made from carry no name, title or text.
#[derive(Debug, Serialize)]
pub struct Summary {
    summary_version: u8,
    generated_at: DateTime<Utc>,
    window_days: u32,
    projects: Vec<Project>,
    /// The comments of the last `window_days`, newest first.
    activity: Vec<Activity>,
}

#[derive(Debug, Serialize)]
struct Project {
    id: i64,
    drafts: u32,
    published: u32,
    /// The published findings.
    by_severity: BySeverity,
    /// The comments of the last `window_days`, by what their author is.
    comments: ByRole,
    last_activity: Option<DateTime<Utc>>,
}

#[derive(Debug, Default, Serialize)]
struct BySeverity {
    low: u32,
    medium: u32,
    elevated: u32,
    high: u32,
    extreme: u32,
}

impl BySeverity {
    fn count(&mut self, severity: Severity) {
        *match severity {
            Severity::Low => &mut self.low,
            Severity::Medium => &mut self.medium,
            Severity::Elevated => &mut self.elevated,
            Severity::High => &mut self.high,
            Severity::Extreme => &mut self.extreme,
        } += 1;
    }
}

#[derive(Debug, Default, Serialize)]
struct ByRole {
    manager: u32,
    staff: u32,
    client: u32,
}

impl ByRole {
    fn count(&mut self, role: UserRole) {
        *match role {
            UserRole::Manager => &mut self.manager,
            UserRole::Staff => &mut self.staff,
            UserRole::Client => &mut self.client,
        } += 1;
    }
}

#[derive(Debug, Serialize)]
struct Activity {
    at: DateTime<Utc>,
    project_id: i64,
    finding_id: i64,
    by: UserRole,
}

#[derive(DerivePartialModel)]
#[sea_orm(entity = "projects::Entity")]
struct ProjectRow {
    id: i64,
}

#[derive(DerivePartialModel)]
#[sea_orm(entity = "findings::Entity")]
struct FindingRow {
    id: i64,
    project_id: i64,
    status: FindingStatus,
    severity: Severity,
    updated_at: DateTimeWithTimeZone,
}

#[derive(DerivePartialModel)]
#[sea_orm(entity = "comments::Entity")]
struct CommentRow {
    finding_id: i64,
    user_id: i64,
    created_at: DateTimeWithTimeZone,
}

#[derive(DerivePartialModel)]
#[sea_orm(entity = "users::Entity")]
struct AuthorRow {
    id: i64,
    role: String,
}

/// A time as the summary carries it: UTC, to the second.
fn instant(at: DateTimeWithTimeZone) -> DateTime<Utc> {
    at.with_timezone(&Utc).trunc_subsecs(0)
}

impl Summary {
    /// The instance as it is at one moment.
    ///
    /// # Errors
    /// When a query fails, a row points at one that is not there, or there
    /// are more projects than a summary carries.
    pub async fn of(db: &DatabaseConnection, now: DateTime<Utc>, window_days: u32) -> Result<Self> {
        // One snapshot, so that every comment has its finding and every
        // finding its project.
        let txn = db
            .begin_with_config(
                Some(IsolationLevel::RepeatableRead),
                Some(AccessMode::ReadOnly),
            )
            .await?;
        let project_rows = projects::Entity::find()
            .order_by_asc(projects::Column::Id)
            .into_partial_model::<ProjectRow>()
            .all(&txn)
            .await?;
        let finding_rows = findings::Entity::find()
            .into_partial_model::<FindingRow>()
            .all(&txn)
            .await?;
        let comment_rows = comments::Entity::find()
            .order_by_desc(comments::Column::CreatedAt)
            .order_by_desc(comments::Column::Id)
            .into_partial_model::<CommentRow>()
            .all(&txn)
            .await?;
        let author_rows = users::Entity::find()
            .into_partial_model::<AuthorRow>()
            .all(&txn)
            .await?;
        txn.commit().await?;
        if project_rows.len() > MAX_PROJECTS {
            return Err(Error::string(&format!(
                "this instance has {} projects, and a summary carries at most {MAX_PROJECTS}",
                project_rows.len()
            )));
        }

        let now = now.trunc_subsecs(0);
        let since = now - Duration::days(i64::from(window_days));
        let roles: HashMap<i64, UserRole> = author_rows
            .iter()
            .map(|author| (author.id, UserRole::parse(&author.role)))
            .collect();
        let project_of: HashMap<i64, i64> = finding_rows
            .iter()
            .map(|finding| (finding.id, finding.project_id))
            .collect();
        let mut projects: Vec<Project> = project_rows
            .iter()
            .map(|project| Project {
                id: project.id,
                drafts: 0,
                published: 0,
                by_severity: BySeverity::default(),
                comments: ByRole::default(),
                last_activity: None,
            })
            .collect();
        let position: HashMap<i64, usize> = projects
            .iter()
            .enumerate()
            .map(|(at, project)| (project.id, at))
            .collect();
        let missing = || Error::string("a row points at one that is not there");

        for finding in &finding_rows {
            let project = &mut projects[*position.get(&finding.project_id).ok_or_else(missing)?];
            match finding.status {
                FindingStatus::Draft => project.drafts += 1,
                FindingStatus::Published => {
                    project.published += 1;
                    project.by_severity.count(finding.severity);
                }
            }
            project.last_activity = project.last_activity.max(Some(instant(finding.updated_at)));
        }

        let mut activity = Vec::new();
        for comment in &comment_rows {
            let project_id = *project_of.get(&comment.finding_id).ok_or_else(missing)?;
            let by = *roles.get(&comment.user_id).ok_or_else(missing)?;
            let at = instant(comment.created_at);
            let project = &mut projects[*position.get(&project_id).ok_or_else(missing)?];
            project.last_activity = project.last_activity.max(Some(at));
            if at < since {
                continue;
            }
            project.comments.count(by);
            if activity.len() < ACTIVITY_LIMIT {
                activity.push(Activity {
                    at,
                    project_id,
                    finding_id: comment.finding_id,
                    by,
                });
            }
        }

        Ok(Self {
            summary_version: 1,
            generated_at: now,
            window_days,
            projects,
            activity,
        })
    }
}

pub struct InstanceSummary;
#[async_trait]
impl Task for InstanceSummary {
    fn task(&self) -> TaskInfo {
        TaskInfo {
            name: "summary".to_string(),
            detail: "Print what this instance holds as one line of JSON, for the host it runs on: ids, counts and times, and no name, title or text.\nUsage:\ncargo run task summary [days:7]".to_string(),
        }
    }
    async fn run(&self, app_context: &AppContext, vars: &task::Vars) -> Result<()> {
        let window_days = match vars.args() {
            [] => DEFAULT_WINDOW_DAYS,
            [only] => only
                .strip_prefix("days:")
                .filter(|days| days.bytes().all(|digit| digit.is_ascii_digit()))
                .and_then(|days| days.parse().ok())
                .filter(|days| (1..=MAX_WINDOW_DAYS).contains(days))
                .ok_or_else(|| {
                    Error::string(&format!(
                        "summary takes `days:<1 to {MAX_WINDOW_DAYS}>` and nothing else, not `{only}`"
                    ))
                })?,
            _ => {
                return Err(Error::string(
                    "summary takes `days:<n>` and nothing else",
                ))
            }
        };

        let summary = Summary::of(&app_context.db, Utc::now(), window_days).await?;
        println!("{}", serde_json::to_string(&summary)?);
        Ok(())
    }
}
