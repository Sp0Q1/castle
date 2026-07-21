//! Findings are the deliverables of a project. Each one carries the four report
//! sections (description, technical description, impact, recommendation) plus a
//! severity and a draft/published status. Access rules enforced here:
//!
//! * write / edit / publish — project staff (a "staff" member, the author, or
//!   management)
//! * read — management and staff see everything; clients see only *published*
//!   findings
use loco_rs::prelude::*;
use serde::{Deserialize, Serialize};

use crate::models::_entities::{comments, findings, project_members, projects, users};
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
    /// low | medium | elevated | high | extreme (defaults to "medium").
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

fn is_valid_severity(value: &str) -> bool {
    matches!(value, "low" | "medium" | "elevated" | "high" | "extreme")
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

async fn load_project(ctx: &AppContext, id: i32) -> Result<projects::Model> {
    projects::Entity::find_by_id(id)
        .one(&ctx.db)
        .await?
        .ok_or_else(|| Error::NotFound)
}

async fn load_finding(ctx: &AppContext, id: i32) -> Result<findings::Model> {
    findings::Entity::find_by_id(id)
        .one(&ctx.db)
        .await?
        .ok_or_else(|| Error::NotFound)
}

/// Staff (a "staff" member of the project, or any manager) may read drafts;
/// clients may not.
fn is_privileged(user: &users::Model, membership: &Option<project_members::Model>) -> bool {
    user.is_manager() || membership.as_ref().is_some_and(|m| m.role == "staff")
}

#[debug_handler]
pub async fn create(
    CurrentUser(user): CurrentUser,
    Path(project_id): Path<i32>,
    State(ctx): State<AppContext>,
    Json(params): Json<CreateFindingParams>,
) -> Result<Response> {
    let project = load_project(&ctx, project_id).await?;
    let membership = project_members::Model::find_membership(&ctx.db, project.id, user.id).await?;

    let is_staff_member = membership.as_ref().is_some_and(|m| m.role == "staff");
    let is_owner_manager = user.is_manager() && project.created_by == user.id;
    let is_manager_member = user.is_manager() && membership.is_some();
    if !(is_staff_member || is_owner_manager || is_manager_member) {
        return unauthorized("only staff assigned to this project can write findings");
    }

    let CreateFindingParams {
        title,
        finding_type,
        description,
        technical_description,
        impact,
        recommendation,
        severity,
    } = params;

    if let Some(severity) = &severity {
        if !is_valid_severity(severity) {
            return bad_request("severity must be one of: low, medium, elevated, high, extreme");
        }
    }

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
    Path(project_id): Path<i32>,
    State(ctx): State<AppContext>,
) -> Result<Response> {
    let project = load_project(&ctx, project_id).await?;
    let membership = project_members::Model::find_membership(&ctx.db, project.id, user.id).await?;

    if !(user.is_manager() || membership.is_some()) {
        return unauthorized("you do not have access to this project");
    }

    let mut items = findings::Model::list_for_project(&ctx.db, project.id).await?;
    if !is_privileged(&user, &membership) {
        items.retain(findings::Model::is_published);
    }

    let response: Vec<FindingResponse> = items.iter().map(FindingResponse::new).collect();
    format::json(response)
}

#[debug_handler]
pub async fn show(
    CurrentUser(user): CurrentUser,
    Path(finding_id): Path<i32>,
    State(ctx): State<AppContext>,
) -> Result<Response> {
    let finding = load_finding(&ctx, finding_id).await?;
    let membership =
        project_members::Model::find_membership(&ctx.db, finding.project_id, user.id).await?;

    if !(user.is_manager() || membership.is_some()) {
        return unauthorized("you do not have access to this finding");
    }
    // Drafts are invisible to clients — behave as if the finding does not exist.
    if !is_privileged(&user, &membership) && !finding.is_published() {
        return Err(Error::NotFound);
    }

    let author = users::Entity::find_by_id(finding.author_id)
        .one(&ctx.db)
        .await?
        .ok_or_else(|| Error::NotFound)?;

    let comment_models = comments::Model::list_for_finding(&ctx.db, finding.id).await?;
    let mut comment_responses = Vec::with_capacity(comment_models.len());
    for comment in &comment_models {
        if let Some(commenter) = users::Entity::find_by_id(comment.user_id)
            .one(&ctx.db)
            .await?
        {
            comment_responses.push(CommentResponse::new(comment, &commenter));
        }
    }

    format::json(FindingDetailResponse::new(
        &finding,
        &author,
        comment_responses,
    ))
}

#[debug_handler]
pub async fn update(
    CurrentUser(user): CurrentUser,
    Path(finding_id): Path<i32>,
    State(ctx): State<AppContext>,
    Json(params): Json<UpdateFindingParams>,
) -> Result<Response> {
    let finding = load_finding(&ctx, finding_id).await?;
    let membership =
        project_members::Model::find_membership(&ctx.db, finding.project_id, user.id).await?;

    let is_author = finding.author_id == user.id;
    let is_staff_member = membership.as_ref().is_some_and(|m| m.role == "staff");
    if !(is_author || is_staff_member || user.is_manager()) {
        return unauthorized("only project staff can edit findings");
    }

    if let Some(severity) = &params.severity {
        if !is_valid_severity(severity) {
            return bad_request("severity must be one of: low, medium, elevated, high, extreme");
        }
    }

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
        severity,
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
    Path(finding_id): Path<i32>,
    State(ctx): State<AppContext>,
) -> Result<Response> {
    let finding = load_finding(&ctx, finding_id).await?;
    let membership =
        project_members::Model::find_membership(&ctx.db, finding.project_id, user.id).await?;

    let is_author = finding.author_id == user.id;
    let is_staff_member = membership.as_ref().is_some_and(|m| m.role == "staff");
    if !(is_author || is_staff_member || user.is_manager()) {
        return unauthorized("only project staff can publish findings");
    }

    let mut item = finding.into_active_model();
    item.status = Set(crate::models::findings::STATUS_PUBLISHED.to_string());
    let finding = item.update(&ctx.db).await?;
    format::json(FindingResponse::new(&finding))
}

#[debug_handler]
pub async fn unpublish(
    CurrentUser(user): CurrentUser,
    Path(finding_id): Path<i32>,
    State(ctx): State<AppContext>,
) -> Result<Response> {
    let finding = load_finding(&ctx, finding_id).await?;
    let membership =
        project_members::Model::find_membership(&ctx.db, finding.project_id, user.id).await?;

    let is_author = finding.author_id == user.id;
    let is_staff_member = membership.as_ref().is_some_and(|m| m.role == "staff");
    if !(is_author || is_staff_member || user.is_manager()) {
        return unauthorized("only project staff can unpublish findings");
    }

    // Reverting to draft hides it from clients again.
    let mut item = finding.into_active_model();
    item.status = Set(crate::models::findings::STATUS_DRAFT.to_string());
    let finding = item.update(&ctx.db).await?;
    format::json(FindingResponse::new(&finding))
}

#[debug_handler]
pub async fn remove(
    CurrentUser(user): CurrentUser,
    Path(finding_id): Path<i32>,
    State(ctx): State<AppContext>,
) -> Result<Response> {
    let finding = load_finding(&ctx, finding_id).await?;
    let membership =
        project_members::Model::find_membership(&ctx.db, finding.project_id, user.id).await?;

    let is_author = finding.author_id == user.id;
    let is_staff_member = membership.as_ref().is_some_and(|m| m.role == "staff");
    if !(is_author || is_staff_member || user.is_manager()) {
        return unauthorized("only project staff can delete findings");
    }

    // Remove dependent comments first so nothing is orphaned (robust even when
    // SQLite foreign-key cascade enforcement is off).
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
