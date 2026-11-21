//! Findings are the deliverables of a project. Each one carries the four report
//! sections (description, technical description, impact, recommendation) plus a
//! severity and a draft/published status. Access rules enforced here:
//!
//! * write / edit / publish — whoever has [`Access::Staff`] on the project
//! * read — staff see everything; with [`Access::Client`] only *published*
//!   findings exist
use loco_rs::prelude::*;
use serde::{Deserialize, Serialize};

use super::access;
use crate::models::_entities::{comments, findings, projects, users};
use crate::models::findings::{FindingStatus, Severity};
use crate::models::project_members::Access;
use crate::security::CurrentUser;
use crate::validation::{self, MAX_LABEL, MAX_SECTION, MAX_TITLE};
use crate::views::comment::CommentResponse;
use crate::views::finding::{FindingDetailResponse, FindingResponse};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CreateFindingParams {
    pub title: String,
    #[serde(default)]
    pub finding_type: Option<String>,
    pub description: String,
    pub technical_description: String,
    pub impact: String,
    pub recommendation: String,
    /// Defaults to `medium` when absent.
    #[serde(default)]
    pub severity: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct UpdateFindingParams {
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub finding_type: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub technical_description: Option<String>,
    #[serde(default)]
    pub impact: Option<String>,
    #[serde(default)]
    pub recommendation: Option<String>,
    #[serde(default)]
    pub severity: Option<String>,
}

/// Bounds every user-supplied text field, shared by create and update. Only
/// fields actually present are checked, so a partial update stays partial.
///
/// Report content is *never* rewritten here — a finding may legitimately quote
/// an exploit payload verbatim. See [`crate::validation`] for why.
fn validate_finding_text(
    title: Option<&str>,
    finding_type: Option<&str>,
    sections: [(&str, Option<&str>); 4],
) -> Result<()> {
    if let Some(title) = title {
        validation::required_text("title", title, MAX_TITLE)?;
    }
    if let Some(finding_type) = finding_type {
        validation::text("finding_type", finding_type, MAX_LABEL)?;
    }
    for (field, value) in sections {
        if let Some(value) = value {
            validation::text(field, value, MAX_SECTION)?;
        }
    }
    Ok(())
}

async fn load_project(ctx: &AppContext, id: i64) -> Result<projects::Model> {
    projects::Entity::find_by_id(id)
        .one(&ctx.db)
        .await?
        .ok_or_else(|| Error::NotFound)
}

async fn load_finding(ctx: &AppContext, id: i64) -> Result<findings::Model> {
    findings::Entity::find_by_id(id)
        .one(&ctx.db)
        .await?
        .ok_or_else(|| Error::NotFound)
}

/// Writing to a project is for its staff. A client on it gets **403**: they
/// know the project. Authorship grants nothing on its own.
fn require_staff(access: Access, action: &str) -> Result<()> {
    match access {
        Access::Staff => Ok(()),
        Access::Client => Err(crate::security::forbidden(action)),
    }
}

/// Changing an *existing* finding is for project staff. For a client a draft
/// does not exist (**404**, matching the read path), and a published finding
/// is known and not theirs to change (**403**).
fn require_finding_write(access: Access, finding: &findings::Model, action: &str) -> Result<()> {
    match access {
        Access::Staff => Ok(()),
        Access::Client if finding.is_published() => Err(crate::security::forbidden(action)),
        Access::Client => Err(Error::NotFound),
    }
}

#[debug_handler]
pub async fn create(
    CurrentUser(user): CurrentUser,
    Path(project_id): Path<i64>,
    State(ctx): State<AppContext>,
    Json(params): Json<CreateFindingParams>,
) -> Result<Response> {
    let project = load_project(&ctx, project_id).await?;
    let access = access(&ctx, &user, project.id).await?;
    require_staff(access, "only project staff can write findings")?;

    let CreateFindingParams {
        title,
        finding_type,
        description,
        technical_description,
        impact,
        recommendation,
        severity,
    } = params;

    let severity = severity
        .as_deref()
        .map(Severity::parse)
        .transpose()
        .map_err(Error::BadRequest)?;

    validate_finding_text(
        Some(&title),
        finding_type.as_deref(),
        [
            ("description", Some(description.as_str())),
            (
                "technical_description",
                Some(technical_description.as_str()),
            ),
            ("impact", Some(impact.as_str())),
            ("recommendation", Some(recommendation.as_str())),
        ],
    )?;

    let mut item = findings::ActiveModel {
        project_id: Set(project.id),
        author_id: Set(user.id),
        title: Set(title),
        description: Set(description),
        technical_description: Set(technical_description),
        impact: Set(impact),
        recommendation: Set(recommendation),
        ..Default::default()
    };
    if let Some(severity) = severity {
        item.severity = Set(severity);
    }
    if let Some(finding_type) = finding_type {
        item.finding_type = Set(finding_type);
    }

    let finding = item.insert(&ctx.db).await?;
    format::json(FindingResponse::new(&finding))
}

#[debug_handler]
pub async fn list(
    CurrentUser(user): CurrentUser,
    Path(project_id): Path<i64>,
    State(ctx): State<AppContext>,
) -> Result<Response> {
    let project = load_project(&ctx, project_id).await?;
    let access = access(&ctx, &user, project.id).await?;

    let mut items = findings::Model::list_for_project(&ctx.db, project.id).await?;
    items.retain(|finding| access.sees(finding));

    let response: Vec<FindingResponse> = items.iter().map(FindingResponse::new).collect();
    format::json(response)
}

#[debug_handler]
pub async fn show(
    CurrentUser(user): CurrentUser,
    Path(finding_id): Path<i64>,
    State(ctx): State<AppContext>,
) -> Result<Response> {
    let finding = load_finding(&ctx, finding_id).await?;
    let access = access(&ctx, &user, finding.project_id).await?;
    // A draft does not exist for a client: the same 404 as a project they
    // cannot open, so nothing is disclosed through the status.
    if !access.sees(&finding) {
        return Err(Error::NotFound);
    }

    let author = users::Entity::find_by_id(finding.author_id)
        .one(&ctx.db)
        .await?
        .ok_or_else(|| Error::NotFound)?;

    let comments = comments::Model::list_for_finding_with_authors(&ctx.db, finding.id).await?;
    let comment_responses: Vec<CommentResponse> = comments
        .iter()
        .filter_map(|(comment, author)| {
            author
                .as_ref()
                .map(|author| CommentResponse::new(comment, author))
        })
        .collect();

    format::json(FindingDetailResponse::new(
        &finding,
        &author,
        comment_responses,
    ))
}

#[debug_handler]
pub async fn update(
    CurrentUser(user): CurrentUser,
    Path(finding_id): Path<i64>,
    State(ctx): State<AppContext>,
    Json(params): Json<UpdateFindingParams>,
) -> Result<Response> {
    let finding = load_finding(&ctx, finding_id).await?;
    let access = access(&ctx, &user, finding.project_id).await?;

    require_finding_write(access, &finding, "only project staff can edit findings")?;

    let severity = params
        .severity
        .as_deref()
        .map(Severity::parse)
        .transpose()
        .map_err(Error::BadRequest)?;

    validate_finding_text(
        params.title.as_deref(),
        params.finding_type.as_deref(),
        [
            ("description", params.description.as_deref()),
            (
                "technical_description",
                params.technical_description.as_deref(),
            ),
            ("impact", params.impact.as_deref()),
            ("recommendation", params.recommendation.as_deref()),
        ],
    )?;

    let mut item = finding.into_active_model();
    let UpdateFindingParams {
        title,
        finding_type,
        description,
        technical_description,
        impact,
        recommendation,
        severity: _,
    } = params;
    if let Some(value) = title {
        item.title = Set(value);
    }
    if let Some(value) = finding_type {
        item.finding_type = Set(value);
    }
    if let Some(value) = description {
        item.description = Set(value);
    }
    if let Some(value) = technical_description {
        item.technical_description = Set(value);
    }
    if let Some(value) = impact {
        item.impact = Set(value);
    }
    if let Some(value) = recommendation {
        item.recommendation = Set(value);
    }
    if let Some(value) = severity {
        item.severity = Set(value);
    }

    let finding = item.update(&ctx.db).await?;
    format::json(FindingResponse::new(&finding))
}

#[debug_handler]
pub async fn publish(
    CurrentUser(user): CurrentUser,
    Path(finding_id): Path<i64>,
    State(ctx): State<AppContext>,
) -> Result<Response> {
    let finding = load_finding(&ctx, finding_id).await?;
    let access = access(&ctx, &user, finding.project_id).await?;

    require_finding_write(access, &finding, "only project staff can publish findings")?;

    let mut item = finding.into_active_model();
    item.status = Set(FindingStatus::Published);
    let finding = item.update(&ctx.db).await?;
    format::json(FindingResponse::new(&finding))
}

#[debug_handler]
pub async fn unpublish(
    CurrentUser(user): CurrentUser,
    Path(finding_id): Path<i64>,
    State(ctx): State<AppContext>,
) -> Result<Response> {
    let finding = load_finding(&ctx, finding_id).await?;
    let access = access(&ctx, &user, finding.project_id).await?;

    require_finding_write(
        access,
        &finding,
        "only project staff can unpublish findings",
    )?;

    // Reverting to draft hides it from clients again.
    let mut item = finding.into_active_model();
    item.status = Set(FindingStatus::Draft);
    let finding = item.update(&ctx.db).await?;
    format::json(FindingResponse::new(&finding))
}

#[debug_handler]
pub async fn remove(
    CurrentUser(user): CurrentUser,
    Path(finding_id): Path<i64>,
    State(ctx): State<AppContext>,
) -> Result<Response> {
    let finding = load_finding(&ctx, finding_id).await?;
    let access = access(&ctx, &user, finding.project_id).await?;

    require_finding_write(access, &finding, "only project staff can delete findings")?;

    // Comments are removed explicitly rather than by ON DELETE CASCADE: SQLite
    // cannot alter a constraint in place, so declaring one now would rebuild the
    // table and leave migrated deployments differing from fresh ones.
    comments::Entity::delete_many()
        .filter(comments::Column::FindingId.eq(finding.id))
        .exec(&ctx.db)
        .await?;
    finding.delete(&ctx.db).await?;
    format::empty()
}

pub fn routes() -> Routes {
    Routes::new()
        .prefix("/api")
        .add("/projects/{project_id}/findings", post(create))
        .add("/projects/{project_id}/findings", get(list))
        .add("/findings/{finding_id}", get(show))
        .add("/findings/{finding_id}", put(update))
        .add("/findings/{finding_id}", delete(remove))
        .add("/findings/{finding_id}/publish", post(publish))
        .add("/findings/{finding_id}/unpublish", post(unpublish))
}
